//! Procedural state graph for the phase-A domain model.
//!
//! Here `G_t = (objects, instances)` plus a background colour. Objects are
//! immutable, declared content. Instances are the mutable-by-transition
//! placements (position, ordering) that materialize as a raster view.
//!
//! The compositor semantics bound the graph to a painter's model:
//! the canvas first fills the whole frame with `background`, then each
//! instance paints its (immutable) object over the canvas in *instance order*
//! (later instances are nearer / higher z).
//!
//! This struct is used by both the normative materializer and the direct
//! procedural ingest API; the checkpoint serializes an exact snapshot of it.

use std::collections::BTreeMap;

use crate::{
    error::VoleError,
    object::{Object, ObjectId},
    time::Interval,
};

/// Instance identity in format-v1 index space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InstanceId(pub u32);

/// A placed, orderable instance of an immutable object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    /// Instance identity for transition targeting.
    pub id: InstanceId,
    /// The declared object whose content this paints.
    pub object_id: ObjectId,
    /// Top-left canvas position of the object box.
    pub x: i64,
    pub y: i64,
}

/// The phase-A procedural state.
#[derive(Debug, Clone)]
pub struct State {
    /// Full-frame background fill (a `FILL` semantic).
    background: u8,
    /// Declared immutable objects keyed by object id.
    objects: BTreeMap<ObjectId, Object>,
    /// Live instances in paint order.
    instances: Vec<Instance>,
    /// Sparse persistent overlay painted above all instances (Phase C: sparse
    /// mutation). Keyed by canvas coordinate; a set pixel persists until
    /// overwritten.
    overlay: BTreeMap<(i64, i64), u8>,
    /// Which interval this state snapshot was produced for (diagnostics and
    /// checkpoint anchoring; a fresh state is interval ZERO).
    interval: Interval,
}

impl Default for State {
    fn default() -> Self {
        Self {
            background: 0,
            objects: BTreeMap::new(),
            instances: Vec::new(),
            overlay: BTreeMap::new(),
            interval: Interval::ZERO,
        }
    }
}

impl State {
    /// Fresh empty state at the given interval with the default background.
    pub fn new(interval: Interval) -> Self {
        Self {
            interval,
            ..Self::default()
        }
    }

    /// Interval anchored to this state.
    pub fn interval(&self) -> Interval {
        self.interval
    }

    /// Set the anchoring interval (used by replay stepping).
    pub fn set_interval(&mut self, interval: Interval) {
        self.interval = interval;
    }

    /// Background colour applied each materialized frame.
    pub fn background(&self) -> u8 {
        self.background
    }

    /// Set the background.
    pub fn set_background(&mut self, colour: u8) {
        self.background = colour;
    }

    /// Number of declared objects.
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Number of live instances.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Lookup an object's content reference.
    pub fn object(&self, id: ObjectId) -> Option<&Object> {
        self.objects.get(&id)
    }

    /// Iterate objects in id order.
    pub fn objects(&self) -> impl Iterator<Item = (&ObjectId, &Object)> {
        self.objects.iter()
    }

    /// Iterate instances in paint order.
    pub fn instances(&self) -> impl Iterator<Item = &Instance> {
        self.instances.iter()
    }

    /// Lookup an instance (failure if absent).
    pub fn instance(&self, id: InstanceId) -> Result<&Instance, VoleError> {
        self.instances
            .iter()
            .find(|i| i.id == id)
            .ok_or(VoleError::UnknownInstance)
    }

    /// Declare a new immutable object. Duplicate ids are an error and
    /// conflicting (identical id but different bytes) declarations are also
    /// rejected: identity is exact.
    pub fn declare_object(&mut self, id: ObjectId, object: Object) -> Result<(), VoleError> {
        if self.objects.contains_key(&id) {
            return Err(VoleError::DuplicateId);
        }
        self.objects.insert(id, object);
        Ok(())
    }

    /// Whether the object is present.
    pub fn has_object(&self, id: ObjectId) -> bool {
        self.objects.contains_key(&id)
    }

    /// Create an instance. Drawing order is the append order.
    pub fn create_instance(
        &mut self,
        id: InstanceId,
        object_id: ObjectId,
        x: i64,
        y: i64,
    ) -> Result<(), VoleError> {
        if self.instances.iter().any(|i| i.id == id) {
            return Err(VoleError::DuplicateId);
        }
        if !self.objects.contains_key(&object_id) {
            return Err(VoleError::UnknownObject);
        }
        self.instances.push(Instance {
            id,
            object_id,
            x,
            y,
        });
        Ok(())
    }

    /// Move the instance's object placement to an absolute position.
    pub fn set_position(&mut self, id: InstanceId, x: i64, y: i64) -> Result<(), VoleError> {
        let inst = self
            .instances
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or(VoleError::UnknownInstance)?;
        inst.x = x;
        inst.y = y;
        Ok(())
    }

    /// Define a sparse-overlay point. Points persist until overwritten with a
    /// new value (blinking/strobe courts overwrite the same coordinates).
    /// Overlay pixels are painted above every instance.
    pub fn set_overlay(&mut self, x: i64, y: i64, value: u8) {
        self.overlay.insert((x, y), value);
    }

    /// Push a batch of overlay points in canonical (lexicographically sorted)
    /// order, validating that they are sorted before application (hostile
    /// requirement: non-canonical order is a typed error, not accepted).
    pub fn overlay_batch(&mut self, pts: &[(i64, i64, u8)]) -> Result<(), VoleError> {
        let mut prev: Option<(i64, i64)> = None;
        for (x, y, v) in pts {
            let key = (*x, *y);
            if let Some(p) = prev {
                if key <= p {
                    return Err(VoleError::NonCanonicalEncoding);
                }
            }
            prev = Some(key);
            self.set_overlay(*x, *y, *v);
        }
        Ok(())
    }

    /// Number of live overlay points.
    pub fn overlay_len(&self) -> usize {
        self.overlay.len()
    }

    /// Iterate overlay points in canonical coordinate order (used by the
    /// materializer as the final paint pass).
    pub fn overlay_iter(&self) -> impl Iterator<Item = (i64, i64, u8)> + '_ {
        self.overlay.iter().map(|((x, y), v)| (*x, *y, *v))
    }
}
