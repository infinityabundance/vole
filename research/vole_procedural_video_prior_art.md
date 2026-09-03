# VOLE: Procedural Video Storage and Transport by Deterministic State Materialization

## Broad Prior-Art Technical Disclosure for Non-ML Inverse Proceduralization, Entropy-Native Persistence, and Residual-Governed Structural Search

**Author:** Riaan de Beer  
**Affiliation:** Invariant Forge LLC  
**Status:** Broad prior-art technical disclosure / research architecture  
**Date:** 2026  
**Project name:** **VOLE — Video Object Layer Engine**  
**Related work:** EntropyFS; Drift–Slew Fusion Bootstrap (DSFB); Forensic Residual Framework (FRF)

---

## Abstract

This paper discloses a broad architecture for **procedural video storage and transport** in which raster frames are not assumed to be the primary persisted or transmitted representation of video. Instead, video is represented as a bounded, deterministic evolution of procedural state together with the residual information required to reproduce a target visual observation.

The central abstraction is:

\[
G_{t+1}=\Phi(U,G_t,\Delta_t),
\]

\[
\hat F_t=M(U,G_t,V),
\]

\[
F_t=\hat F_t\oplus_{\rho}R_t,
\]

where \(U\) is a versioned deterministic universe of normative algorithms and tables; \(G_t\) is a bounded **Procedural State Graph** at time \(t\); \(\Phi\) is a deterministic state-transition operator; \(\Delta_t\) is a bounded transition description; \(M\) is a deterministic materializer; \(V\) is a declared view, rasterization, plane, scale, region, or output parameter; \(R_t\) is residual information not reproduced by the procedural state; and \(\oplus_{\rho}\) is an explicitly defined residual algebra.

Under this model, a frame is a **materialized view of persistent mathematical/configurational state**, not necessarily the fundamental stored object. A stream may carry immutable objects, checkpoints, state transitions, transformations, procedural generators, palettes, exact references, sparse changes, motion or affine states, bounded drawing/composition operations, entropy-coded symbol streams, and residuals. A receiver evolves the same deterministic state and materializes raster samples only where and when required. Native procedural sources may enter the representation directly. Raster sources may be converted by an **inverse proceduralization** encoder that searches for the least costly admissible deterministic explanation of the observed video and stores the unexplained remainder as residual information.

One useful lossless objective is:

\[
D^*=
\arg\min_D
\left[
L(D)+L(F\mid D)
\right],
\]

subject to exact reconstruction, bounded decoding, declared dependency limits, and complete physical accounting. Here \(L(D)\) is the cost of the procedural description and \(L(F\mid D)\) is the cost of the exact residual required after that description is materialized. This is related in spirit to minimum-description-length and analysis-by-synthesis ideas, but the disclosed architecture is an explicit bounded video representation system rather than a statistical claim about the intrinsic description length of arbitrary scenes.

The architecture deliberately separates three concerns:

\[
\boxed{\text{VOLE}=\text{normative procedural video representation and materialization}}
\]

\[
\boxed{\text{EntropyFS}=\text{optional representation-aware persistence}}
\]

\[
\boxed{\text{DSFB}=\text{zero-authority structural search intelligence}}
\]

VOLE must decode without DSFB and without EntropyFS. EntropyFS may persist and share immutable procedural objects, states, dictionaries, models, residuals, and equivalent representations across videos. DSFB may observe the residual trajectories of competing procedural hypotheses and use drift, slew, and trust to allocate encoder search effort, but it cannot alter normative reconstruction, make an invalid candidate valid, or override exact final cost selection.

This paper does **not** claim that video entropy disappears, that arbitrary camera footage can be regenerated from a short formula, or that procedural representation defeats information theory. When a bounded procedural model explains little, the residual approaches the information content of the raster source and VOLE must gracefully fall back toward literal or ordinary entropy-coded storage. Nor does this paper claim that procedural graphics, scene graphs, model-based coding, remote drawing orders, object-based video, motion compensation, delta coding, vector animation, or content-addressed storage are new in isolation. Strong prior-art antecedents exist in MPEG-4 BIFS and object coding, VRML/X3D, SVG/SMIL, vector-animation formats, model-based image coding, procedural texture synthesis, dynamic-texture research, remote-display protocols, hybrid video codecs, VCDIFF, content-addressed stores, scalable/derived media systems, and archival codecs.

The disclosed contribution is the **broad combination and architecture**: treat video itself as bounded evolving procedural state; make raster frames a materialization surface; permit arbitrary raster video to enter through inverse proceduralization plus exact residual; use deterministic residual evidence to govern which structural explanations deserve search; support streaming as state replication plus bounded residual innovation; preserve a universal raster fallback; and make every claimed storage, bandwidth, latency, or computational advantage subject to reproducible empirical courts.

---

## Keywords

procedural video; deterministic video; non-ML video; inverse proceduralization; procedural state graph; materialization; entropy-native storage; generative representation; residual coding; structural video compression; state-transition streaming; content-addressed video; scene graph; model-based coding; screen-content coding; remote display; exact reconstruction; DSFB; EntropyFS; Video Object Layer Engine; bounded decoder; non-destructive video; deterministic replay; mathematical video representation; residual-governed search

---

# 1. Purpose and Disclosure Posture

This document is intentionally written as a **wide prior-art technical disclosure**. It does not employ a narrow-wedge strategy and is not restricted to a single codec mode, corpus, visual domain, transform, entropy coder, hardware platform, or storage backend.

Its purpose is to place into the public technical record a broad family of architectures in which:

1. video may be persisted as deterministic procedural state rather than only as a sequence of compressed raster pictures;
2. time may be represented as deterministic evolution of that state rather than only as picture-to-picture prediction;
3. raster samples may be materialized on demand as a view of state;
4. raster video may be converted into that representation through bounded inverse proceduralization;
5. irreducible disagreement remains explicit as residual information;
6. structural and residual information may be entropy coded and content addressed;
7. procedural state may be streamed as checkpoints and transitions;
8. persistent objects may retain identity across frames, videos, edits, renditions, or storage namespaces;
9. DSFB or an equivalent deterministic observer may use residual trajectories to govern search effort without acquiring decoding authority; and
10. equivalent procedural representations may be rewritten later if they materialize the same target more efficiently.

The disclosure is deliberately empirical. Expressibility is not evidence of advantage. A procedural representation is useful only if its complete cost is lower under a declared objective. A state transition is useful only if it is cheaper than an appropriate alternative. A generated basis that requires a residual larger than a conventional coding mode should lose. A DSFB search policy that does not beat a strong deterministic heuristic on search economics should receive no credit.

The system therefore adopts the rule:

> **Store the process when the process is cheaper. Encode what the process cannot explain. Materialize raster samples only when they are required.**

This is a design objective, not an assumption that every video has a compact procedural cause available to the encoder.

---

# 2. The Change in Representation: From Raster Sequence to Procedural State

## 2.1 Conventional raster-centered view

A conventional decoded video is commonly understood as a sequence:

\[
F_0,F_1,F_2,\ldots,F_T,
\]

where each \(F_t\) is a raster picture. Modern codecs exploit extensive redundancy between these pictures, but the normative object produced by decoding remains a picture sequence. Inter prediction, transforms, entropy coding, screen-content tools, global motion, and reference structures are sophisticated mechanisms for representing these raster outputs efficiently.

This paper does not dispute the effectiveness of that architecture.

It asks a different systems question:

> Must the raster frame remain the primary persistent ontology of video?

## 2.2 Procedural-state view

VOLE instead permits the persistent object to be an evolving state:

\[
G_0,G_1,\ldots,G_T,
\]

with:

\[
G_{t+1}=\Phi(U,G_t,\Delta_t).
\]

A raster frame is then a view:

\[
F_t=M(U,G_t,V_t)\oplus_{\rho}R_t.
\]

The state may contain:

- immutable visual objects;
- object instances;
- region identities;
- transforms;
- trajectories;
- drawing or composition operations;
- palettes;
- procedural generators;
- motion fields;
- affine fields;
- exact references;
- sparse mutations;
- dictionaries;
- entropy models;
- residual objects;
- raster fallback objects;
- view-dependent parameters;
- integrity commitments;
- bounded dependency references.

The state is not required to contain semantic objects such as “person,” “car,” or “tree.” A VOLE object may simply be an exact tile, a rectangle copied from an earlier materialization, a recurring raster asset, a palette-defined region, a transformable patch, or a deterministic generated field.

## 2.3 Raster as a materialization surface

The materializer exposes ordinary pixels when an application asks for them.

For a whole frame:

\[
F_t=M(U,G_t,V_{\mathrm{frame}}).
\]

For a tile:

\[
F_{t,i,j}=M(U,G_t,V_{i,j}).
\]

For a crop:

\[
F_{t,\Omega}=M(U,G_t,V_{\Omega}).
\]

For scanout-oriented execution, an implementation may materialize only the region or scanlines that are needed next, provided all normative dependency requirements are satisfied.

The architecture therefore separates:

- **persistent representation** from
- **materialized raster output**.

Today's displays still consume raster samples. VOLE does not remove rasterization from display hardware. It permits rasterization to move later in the pipeline and makes it unnecessary for every full raster frame to exist as the primary stored object.

---

# 3. Prior-Art Landscape

The architecture sits within a large and important body of established work. The following antecedents are not treated as obstacles to acknowledge reluctantly; they are necessary context for understanding what is and is not being disclosed.

## 3.1 Hybrid predictive-transform video coding

H.264/AVC, HEVC, AV1, VVC, and related codecs use sophisticated combinations of intra prediction, inter-picture prediction, motion compensation, variable partitioning, transforms, quantization, entropy coding, loop filtering, screen-content tools, global motion, and reference-picture structures.

VOLE does not claim these techniques as new. They remain valuable candidate representations inside a procedural-state framework. A transform-coded residual may be the best representation for a region. A conventional motion-compensated base may be the best state transition for natural video. The procedural architecture does not require replacing proven coding tools where they win.

## 3.2 MPEG-4 object-based coding, sprites, and BIFS scene description

MPEG-4 is especially important prior art.

MPEG-4 Visual included object-oriented concepts, shape coding, sprites, facial/body animation tools, and composition-aware media. MPEG-4 BIFS (Binary Format for Scenes), formalized in the MPEG-4 scene-description framework, represents dynamic and interactive audiovisual presentations as a scene graph. BIFS supports binary-compressed scene descriptions, 2D/3D graphics, object reuse, animations, and streamed commands that insert, delete, replace, and update scene-graph elements. BIFS-Anim supports efficient streamed changes to scene parameters.

This is strong prior art for:

- scene graphs as audiovisual state;
- streaming scene mutations;
- persistent object identity;
- parameter animation;
- receiver-side composition;
- mixing synthetic and natural media;
- compressed scene descriptions.

VOLE therefore does **not** claim the general idea of transmitting a scene description rather than a flattened raster as new.

The intended distinction is architectural and empirical: VOLE treats procedural state plus residual as a **general video representation target**, including for raster-origin video that did not begin as an authored scene graph. A VOLE encoder may infer bounded procedural explanations from observed raster video and preserve exact reconstruction through residual correction. Conventional raster coding remains a universal fallback inside the same representation.

## 3.3 VRML and X3D

VRML and its successor X3D establish long-standing prior art for storing, retrieving, communicating, and rendering scene graphs and real-time graphics content. X3D is an ISO-standardized scene representation and runtime architecture, with explicit object and scene structure rather than pre-rendered video alone.

VOLE does not claim scene-graph persistence, runtime rendering, or representation of 2D/3D objects as new. It incorporates those ideas into a constrained video materialization model where exact raster targets, residuals, codec-style entropy coding, video checkpoints, bounded dependency graphs, and inverse proceduralization are first-class concerns.

## 3.4 SVG, SMIL, Lottie, Rive, and vector animation

SVG and SMIL support declarative animation of graphical properties and transforms. Modern vector animation formats such as Lottie describe layered vector animation, keyframes, assets, and animatable properties. Rive uses artboards, animation state machines, transitions, and runtime advancement over time.

These systems demonstrate an important fact: for authored vector animation, storing graphical state and temporal rules can be more natural than storing raster frames.

VOLE generalizes the procedural principle beyond vector-authored content by allowing:

- raster and procedural objects in the same state;
- exact residual correction;
- bounded inverse proceduralization of raster-origin sources;
- content-addressed cross-video objects;
- codec-style entropy coding;
- explicit fallback to raster payloads;
- empirical candidate competition.

## 3.5 Procedural texture and graphics synthesis

Procedural graphics have decades of prior art. Perlin's 1985 *An Image Synthesizer* described naturalistic visual complexity constructed from compositions of functions and introduced solid-texture concepts. Procedural modeling subsequently became widespread for textures, terrain, vegetation, cities, particles, materials, and effects.

VOLE does not claim procedural generation of visual content as new.

The relevance is more specific: if a target visual region can be reproduced by a deterministic bounded generator plus a compact residual, the generator is a legitimate representation candidate. If it cannot, the generator loses.

Thus:

\[
F=M(\text{generator},\text{state})\oplus R
\]

is a coding option, not an assertion that natural imagery is intrinsically procedural in a compact way.

## 3.6 Model-based image and video coding

Model-based coding and analysis-by-synthesis research has long investigated transmitting model parameters rather than waveform samples, especially for very-low-bit-rate facial or talking-head video. Aizawa and Huang surveyed 2D and 3D model-based image coding in the 1990s, including facial models and deformable patches.

This is strong prior art for:

- parameterizing visual structure;
- analysis-by-synthesis;
- model parameter transmission;
- receiver-side reconstruction.

VOLE does not claim these concepts in isolation. It broadens the representational substrate away from a domain-specific semantic model toward a finite family of bounded structural hypotheses with exact raster fallback and explicit residual accounting.

## 3.7 Dynamic textures and generative temporal models

Dynamic-texture research modeled and synthesized time-varying phenomena such as waves, smoke, foliage, traffic, and other temporally structured imagery. This is relevant prior art for temporal generative models of video-like data.

VOLE differs by requiring a normative bounded representation, explicit residual correction, hostile-input limits, deterministic reconstruction, and cost competition against ordinary coding modes. A generative model is useful only when the complete description plus residual wins.

## 3.8 Remote desktop and drawing-order protocols

Remote-display systems are particularly relevant to procedural streaming.

Microsoft RDP has historically supported **drawing orders** for operations such as rectangles, lines, polygons, ellipses, text fragments, blits, cached bitmaps, and copying portions of the screen. When drawing orders are not suitable, bitmap updates can be sent. RDP therefore establishes strong prior art for transmitting display operations and cached resources rather than only full raster-frame video.

VOLE does not claim command-based screen updates as new.

The broader disclosed architecture treats such operations as one family within a general procedural video representation that can also encode natural video, residuals, motion, transforms, procedural generators, exact object references, and persistent state with codec-style accounting.

## 3.9 Delta coding

VCDIFF and related delta formats establish COPY/ADD/RUN reconstruction relative to source data. VOLE's one-dimensional and two-dimensional copy, patch, and literal operations are descendants of that broad concept.

The disclosed system specializes delta reconstruction for temporal visual state, spatial coordinates, object identity, region materialization, and video dependency profiles.

## 3.10 Content-addressed storage

Venti and subsequent content-addressed systems establish immutable hash-addressed storage and exact deduplication. Filesystems also provide compression, copy-on-write, snapshots, deduplication, and garbage collection.

VOLE and EntropyFS do not claim content addressing as new. The disclosed combination makes content-addressed objects directly meaningful to the procedural video representation: objects, bases, dictionaries, palettes, models, residuals, checkpoints, and edit components may be shared and independently accounted.

## 3.11 Scalable, derived, and reconfigurable media

H.264/SVC and related scalable coding systems establish spatial, temporal, and quality layering. ISO Base Media File Format mechanisms and derived visual tracks establish relationships between media objects and transformations. MPEG Reconfigurable Video Coding establishes declarative codec configurations as networks of functional units.

VOLE acknowledges all of these as prior art. Its materializer remains intentionally finite, bounded, and non-Turing-complete.

## 3.12 Learned implicit and scene-level video representations

Recent learned video-representation research has represented scenes or sequences with neural implicit models. For example, scene-level deep video compression work has explored modeling an entire scene instead of relying solely on frame-by-frame predictive coding.

VOLE is intentionally non-ML. No neural weights or learned latent field are required. These works are still relevant prior art to the broad idea that a video sequence can be modeled as something other than a conventional raster bitstream.

The distinction is not a claim that non-ML is automatically better. It is a design choice favoring explicit state, deterministic replay, bounded execution, model transparency, and exact candidate validation.

---

# 4. Formal Model

## 4.1 Universe

Let \(U\) denote a versioned normative universe:

\[
U=\{\text{algorithms},\text{tables},\text{transforms},\text{generators},\text{entropy rules},\text{limits}\}.
\]

A conforming decoder knows the meaning of the declared universe version.

The universe may define:

- residual algebras;
- integer transforms;
- interpolation rules;
- rasterization rules;
- combinatorial rank/unrank;
- entropy-coder semantics;
- procedural primitive semantics;
- fixed-point arithmetic;
- hash canonicalization;
- default models;
- bounded generators.

A universe must not depend on unspecified floating-point behavior or external mutable state for normative reconstruction.

## 4.2 Procedural State Graph

At time \(t\), video state is:

\[
G_t=(O_t,I_t,T_t,D_t,C_t,R_t,M_t),
\]

where, illustratively:

- \(O_t\): immutable objects and object identities;
- \(I_t\): object instances;
- \(T_t\): transforms, trajectories, and motion states;
- \(D_t\): deterministic dynamics or generator states;
- \(C_t\): composition, ordering, masks, and region relationships;
- \(R_t\): residual objects or bindings;
- \(M_t\): entropy models, palettes, dictionaries, and materialization metadata.

Not every implementation must expose these exact tuples. The disclosed concept is a finite, bounded state whose semantics are sufficient to reconstruct declared visual outputs.

## 4.3 State transitions

A transition descriptor \(\Delta_t\) changes state:

\[
G_{t+1}=\Phi(U,G_t,\Delta_t).
\]

Examples:

```text
CREATE_OBJECT
DROP_OBJECT
INSTANCE_OBJECT
DELETE_INSTANCE
SET_TRANSFORM
DELTA_TRANSFORM
SET_TRAJECTORY
ADVANCE_TRAJECTORY
SET_PALETTE
PATCH_PALETTE
COPY_REGION
MOVE_REGION
PATCH_REGION
SET_GENERATOR_STATE
ADVANCE_GENERATOR
BIND_RESIDUAL
REBASE_REGION
SET_VIEW
```

These are conceptual families, not a frozen opcode table.

The critical property is bounded deterministic semantics.

## 4.4 Materialization

A materializer produces a view:

\[
X=M(U,G_t,V).
\]

The view \(V\) may specify:

- full frame;
- tile;
- rectangle;
- plane;
- mip/scale;
- color output;
- layer;
- eye/viewpoint for an explicitly supported procedural profile;
- scanline or scanout range.

For a canonical raster target \(F_t^*\), lossless materialization requires:

\[
M(U,G_t,V_{\mathrm{canonical}})\oplus_{\rho}R_t=F_t^*.
\]

## 4.5 Explicit residual algebra

The residual operator is never implied.

Possible residual semantics include:

```text
XOR
MODULAR_ADD
SIGNED_DELTA
SPARSE_OVERWRITE
TRANSFORM_RECONSTRUCTION
COMMAND_RESIDUAL
PALETTE_EXCEPTION
RAW_REPLACEMENT
```

The bitstream identifies the operation precisely.

## 4.6 Procedural description length

For candidate procedural explanation \(H\):

\[
C(H)=L(H)+L(F\mid H).
\]

A richer objective may include compute and dependency penalties:

\[
J(H)=
B(H)
+\lambda_e E(H)
+\lambda_d D(H)
+\lambda_m M(H)
+\lambda_i I(H)
+\lambda_h H_d(H),
\]

where:

- \(B\): persisted or transmitted bits;
- \(E\): encoder work;
- \(D\): decoder/materializer work;
- \(M\): memory;
- \(I\): dependent fetch or I/O cost;
- \(H_d\): dependency depth.

The encoder selects the cheapest admissible candidate under the declared profile.

No structural mechanism is presumed to win.

---

# 5. Two Entry Paths: Native Procedural and Inverse Proceduralized

## 5.1 Native procedural sources

Some sources already possess structured state before rasterization:

- vector animation;
- user-interface composition;
- game scenes;
- simulation outputs;
- CAD/visualization;
- dashboards;
- motion graphics;
- procedurally generated animation;
- deterministic render pipelines.

Where source semantics are available, VOLE may ingest them directly rather than discarding structure into pixels and then attempting to recover it.

A pipeline may therefore be:

```text
source state
   ↓
VOLE procedural state
   ↓
VOLE transitions
   ↓
materializer
   ↓
raster only when needed
```

This path can preserve information that a capture-after-rasterization workflow destroys.

## 5.2 Raster-origin sources

Camera video, decoded legacy video, screen capture, and other sources may arrive only as raster observations.

For these, the encoder performs **inverse proceduralization**:

```text
target raster
   ↓
candidate structural hypotheses
   ↓
materialize each candidate
   ↓
measure exact residual
   ↓
complete cost court
   ↓
select procedural description + residual
```

The encoder asks:

> Which bounded deterministic process explains the greatest amount of this observation for the lowest complete cost?

This is not semantic understanding by necessity. A hypothesis may simply be:

- “same region as before”;
- “this rectangle moved by \((dx,dy)\)”;
- “these 17 pixels changed”;
- “this tile is an exact object already known”;
- “this region is palette indexed”;
- “this affine transformation plus residual reconstructs the target”;
- “this deterministic generator plus residual is cheaper”;
- “none of the above; use transform/rANS/raw.”

## 5.3 Exact candidate validation

For lossless coding, every candidate must satisfy:

\[
\operatorname{Materialize}(H)+R=F^*.
\]

A candidate generator is never trusted because it looks plausible.

The candidate is authoritative only after exact reconstruction validation.

---

# 6. Procedural State Graph Semantics

## 6.1 Persistent identity

A key design objective is to preserve object identity across time.

In a raster-only view, a toolbar may appear as newly reconstructed samples in every frame. In a procedural view:

```text
Object toolbar_17
Instance toolbar_17 at (0,0)
```

may remain alive for thousands of transitions.

Only its transform, visibility, clipping, or residual state need change.

This permits:

- identity-based reuse;
- compact transition streams;
- persistent cache residency;
- cross-video sharing where exact identity holds;
- non-destructive editing;
- offline structural re-optimization.

## 6.2 Objects need not be semantic

A state graph does not require semantic computer vision.

An object may be:

- a 64x64 exact tile;
- a 412x28 raster strip;
- a glyph atlas;
- a palette;
- a transformable screen region;
- a reusable residual block;
- an entropy dictionary;
- a procedural field;
- a previously materialized region.

The encoder may discover object identity purely through hashes, equality, deterministic matching, residual economics, and temporal persistence.

## 6.3 Composition

A frame may be composed from ordered layers or bounded operations:

```text
CLEAR
DRAW_OBJECT
COPY_RECT
FILL_RECT
BLIT
APPLY_PALETTE
APPLY_TRANSFORM
APPLY_MASK
PATCH
APPLY_RESIDUAL
```

The composition model must define:

- coordinate system;
- clipping;
- overlap;
- ordering;
- alpha semantics where applicable;
- arithmetic;
- color space;
- output dimensions.

## 6.4 State immutability versus instance mutability

One useful implementation distinction is:

- immutable content objects;
- mutable-by-transition instance state.

An immutable object can be content addressed safely. A transition modifies the instance graph rather than the object itself.

This is compatible with transactional persistence and cross-stream object sharing.

---

# 7. Procedural Transition Language

The transition language is central to procedural video streaming.

## 7.1 Transition classes

A broad transition family may include:

### Object lifecycle

```text
OBJECT_DECLARE
OBJECT_BIND
OBJECT_RELEASE
```

### Instance lifecycle

```text
INSTANCE_CREATE
INSTANCE_DELETE
INSTANCE_ENABLE
INSTANCE_DISABLE
```

### Geometry and transforms

```text
SET_POSITION
DELTA_POSITION
SET_SCALE
SET_ROTATION
SET_AFFINE
SET_CLIP
SET_ZORDER
```

### Dynamics

```text
SET_VELOCITY
SET_ACCELERATION
SET_SPLINE
ADVANCE_PARAMETRIC
SET_GENERATOR
ADVANCE_GENERATOR
```

### Raster structural operations

```text
COPY_RECT
MOVE_RECT
FILL_RECT
PATCH_RECT
LITERAL_RECT
EXACT_REGION_REF
```

### Residual state

```text
SET_RESIDUAL
PATCH_RESIDUAL
CLEAR_RESIDUAL
REBASE
```

### Model/dictionary state

```text
SET_ENTROPY_MODEL
SET_DICTIONARY
SET_PALETTE
PATCH_PALETTE
```

The final language should remain intentionally smaller than a general graphics programming language.

## 7.2 Non-Turing-complete execution

VOLE procedural state is not an invitation to embed arbitrary scripts.

The materializer should be:

- finite;
- bounded;
- deterministic;
- statically or dynamically resource limited;
- auditable;
- hostile-input safe.

A generator may have a bounded update rule, but no descriptor may create unbounded loops, recursion, arbitrary system calls, or unbounded memory growth.

## 7.3 Transition determinism

Every transition must define:

- integer width;
- fixed-point format where applicable;
- rounding;
- overflow;
- ordering;
- source-state requirements;
- failure behavior;
- operation budget.

---

# 8. Streaming as Replicated Procedural State

## 8.1 Packet ontology

A procedural video stream may be organized around five primary packet classes:

```text
OBJECT
CHECKPOINT
TRANSITION
RESIDUAL
INTEGRITY
```

Optional model, dictionary, metadata, and index packets may be explicit subtypes.

### OBJECT

Introduces immutable reusable content.

### CHECKPOINT

Defines a bounded self-sufficient procedural state or restart state.

### TRANSITION

Advances the state from one time interval to another.

### RESIDUAL

Carries information not reproduced by the current procedural explanation.

### INTEGRITY

Commits to object, state, transition, or reconstructed output hashes.

## 8.2 Structural innovation rate

The desired streaming behavior is that bandwidth responds to **new information and structural change**, rather than scaling mechanically with raster resolution and frame rate when the scene is structurally static.

This is a hypothesis, not a universal law.

A static slide with a moving cursor might require:

- one persistent slide object;
- occasional cursor state updates;
- small residuals.

A high-entropy noisy camera stream may require large residual payloads approaching ordinary raster-coded cost.

The empirical question is:

\[
\text{How closely can transmitted bits track structural innovation rather than repeated rasterization?}
\]

## 8.3 Checkpoints

Long procedural chains can create latency, loss propagation, and seek cost.

VOLE therefore supports bounded checkpoints:

\[
G_k=\operatorname{Checkpoint}(k).
\]

Subsequent states may be:

\[
G_{k+n}=\Phi^n(G_k,\Delta_{k:k+n}).
\]

Checkpoint cadence creates an explicit trade-off:

\[
\text{frequent checkpoints}
\Rightarrow
\text{better resilience and access, greater overhead}
\]

versus:

\[
\text{sparse checkpoints}
\Rightarrow
\text{better density, larger dependency horizon}.
\]

## 8.4 Receiver caches

Because object identity persists, a receiver may cache:

- immutable visual objects;
- dictionaries;
- entropy models;
- palettes;
- procedural generator tables;
- checkpoints;
- frequently reused regions.

A stream can reference a compact local index instead of retransmitting full hashes on every use.

## 8.5 Missing state

A conforming normative decoder must fail closed on a missing mandatory dependency unless the profile defines a separate concealment behavior.

Error concealment is not normative reconstruction.

---

# 9. Bounded Representation Families

The state graph and transition model operate over a finite representation vocabulary.

## 9.1 RAW

```text
RAW_REF(object_id)
INLINE_RAW(bytes)
```

Universal escape hatch.

## 9.2 Entropy-coded literal

```text
RANS_REF(model_id, payload_id)
```

or an equivalent normative entropy coder.

## 9.3 FILL

```text
FILL(value)
FILL_RECT(...)
```

## 9.4 Exact object reference

```text
EXACT_FRAME_REF
EXACT_TILE_REF
EXACT_REGION_REF
EXACT_OBJECT_REF
```

## 9.5 Temporal/spatial base plus residual

```text
BASE_REF(...)
RESIDUAL(...)
```

## 9.6 Motion and affine state

```text
TRANSLATION
MOTION_FIELD
AFFINE
GLOBAL_TRANSFORM
```

## 9.7 Sparse mutation

\[
R=(S,\Delta_S).
\]

Support may be stored as:

- sorted indices;
- runs;
- bitmaps;
- sparse blocks;
- combinatorial ranks;
- context-coded occupancy.

## 9.8 Palette state

```text
PALETTE
PALETTE_PATCH
INDEX_STREAM
```

## 9.9 Two-dimensional copy/move

```text
COPY_RECT
MOVE_RECT
PATCH_RECT
LITERAL_RECT
```

## 9.10 Sequence delta

Bounded COPY/LITERAL/RUN commands over a prior or dictionary source.

## 9.11 Transform residual

```text
TRANSFORM_RESIDUAL {
    base,
    transform,
    quantizer_or_lossless_mode,
    coefficients
}
```

## 9.12 Procedural generator

```text
GENERATOR {
    generator_id,
    seed_or_state,
    parameters,
    coordinate,
    residual
}
```

A generator never gets special epistemic status. It is simply another candidate.

## 9.13 Parametric dynamics

```text
POSITION(t)
SPLINE(t)
AFFINE(t)
PALETTE(t)
GENERATOR_STATE(t)
```

A compact trajectory may replace repeated per-frame parameter updates when exact deterministic evaluation is cheaper.

For example:

\[
x(t)=x_0+vt+\frac12at^2
\]

may encode an exact integer/fixed-point trajectory over a bounded interval if the chosen arithmetic reproduces the intended positions exactly.

## 9.14 Raster replacement

A procedural region may be abandoned:

```text
REBASE_RAW
```

when residual cost or regime change makes the procedural hypothesis uneconomic.

This prevents path dependence from forcing a stale model.

---

# 10. Entropy-Native Mathematics: What Is Actually Being Compressed

The phrase “compress the entropy math” is evocative but requires technical precision.

Entropy is a measure, not a payload.

VOLE instead compresses:

1. the deterministic generative/configurational description;
2. its state transitions;
3. the symbols necessary to parameterize that state;
4. the residual information that the deterministic description cannot reproduce.

Thus:

\[
\text{stored information}
=
\text{procedure/state}
+
\text{transitions}
+
\text{residual}.
\]

The goal is for repeated raster detail to be represented once as state when possible.

If a region is fully procedural:

\[
R=0.
\]

If the procedure explains most of it:

\[
|R|\ll|F|.
\]

If the procedure explains little:

\[
|R|\rightarrow|F|
\]

and the encoder should choose an ordinary coding mode instead.

This preserves information-theoretic honesty.

---

# 11. Inverse Proceduralization as Search

## 11.1 Candidate hypotheses

For a target region \(F\), the encoder may evaluate:

\[
H_0=\text{RAW}
\]

\[
H_1=\text{exact persistent object}
\]

\[
H_2=\text{previous state unchanged}
\]

\[
H_3=\text{translation}
\]

\[
H_4=\text{affine transform}
\]

\[
H_5=\text{sparse mutation}
\]

\[
H_6=\text{copy/move composition}
\]

\[
H_7=\text{palette state}
\]

\[
H_8=\text{parametric dynamics}
\]

\[
H_9=\text{generated basis}
\]

\[
H_{10}=\text{transform-coded residual}.
\]

For each candidate:

\[
\hat F_k=M(H_k)
\]

and:

\[
R_k=F\ominus_{\rho_k}\hat F_k.
\]

Complete cost is then measured.

## 11.2 The residual as evidence

The residual is not merely discarded after coding.

Its magnitude, support, persistence, topology, sign structure, entropy, and temporal trajectory provide evidence about whether the current procedural explanation remains appropriate.

A translation hypothesis with a shrinking residual may be gaining explanatory power.

A formerly exact object whose residual suddenly expands may have undergone a structural regime change.

A sparse hypothesis whose support density gradually increases may be approaching rebase.

This is where DSFB becomes structurally relevant.

---

# 12. DSFB as Zero-Authority Procedural Search Intelligence

## 12.1 Authority separation

DSFB is not part of normative decoding.

It may not:

- change reconstructed samples;
- change generator semantics;
- bypass candidate validation;
- declare a lossy candidate lossless;
- override exact final cost comparison;
- be required by the receiver.

It may only affect which candidates the encoder spends effort evaluating.

## 12.2 Procedural hypothesis observers

A useful DSFB bank may track families such as:

```text
UNCHANGED
EXACT_REF
TRANSLATION
AFFINE
SPARSE
COPY_RECT
PALETTE
PARAMETRIC
GENERATOR
TRANSFORM
RAW
```

A utility measurement may be:

\[
y_k=
\operatorname{clamp}_{[0,1]}
\left(
1-
\frac{\log(1+J_k)}{\log(1+J_{\mathrm{raw}})}
\right).
\]

DSFB receives deterministic measurements of candidate usefulness.

It does not interpret \(y_k\) as a probability.

## 12.3 Drift and slew

A useful interpretation is:

- \(\phi\): current quality of a procedural explanation;
- \(\omega\): gradual drift in its explanatory power;
- \(\alpha\): slew indicating rapid regime change.

Suppose a region follows a translation model for 500 frames.

Residual remains small.

Then deformation begins and the residual expands rapidly.

A high-slew event can trigger:

1. broader hypothesis search;
2. temporary revival of previously weak families;
3. deeper motion or affine search;
4. split/repartition exploration;
5. local rebase;
6. eventual narrowing after a new stable explanation is found.

## 12.4 Local rebaselining

A scene should not require a whole-frame reset because one region changed regime.

The state graph supports local rebaselining:

```text
REBASE_REGION(region_id, new_basis)
```

while unrelated regions retain their state.

## 12.5 Deterministic sentinels

Search must avoid stale exploitation.

A deterministic policy may:

- always test RAW;
- always test incumbent;
- always test one cheap universal alternative;
- test top \(N\) trusted hypotheses;
- rotate a sentinel hypothesis every \(M\) regions;
- test all hypotheses after strong slew.

No random bandit is required.

## 12.6 Empirical DSFB claim

The primary DSFB question is:

\[
N_{\mathrm{DSFB}}<N_{\mathrm{exhaustive}}
\]

while:

\[
J_{\mathrm{DSFB}}\le J_{\mathrm{exhaustive}}+\epsilon.
\]

If a fixed heuristic achieves the same result more cheaply, the DSFB hypothesis fails for that workload.

---

# 13. Resolution, View, and Partial Rasterization

## 13.1 Procedural portions may be resolution-independent

A rectangle, vector path, palette, affine transform, parametric trajectory, or procedural field may not intrinsically belong to 1080p.

It can potentially be materialized at:

\[
720p,\ 1080p,\ 4K,\ 8K
\]

under a declared view transform.

Raster residuals, however, may remain tied to their sampling grid.

VOLE therefore permits hybrid state:

```text
resolution-independent structural state
+
resolution-bound raster residual
```

The system must never imply that a low-resolution raster residual magically contains high-resolution detail.

## 13.2 Materialization profiles

A stream may define:

- canonical raster view;
- optional derived views;
- scale-independent objects;
- view-specific residual layers;
- deterministic scaling filters;
- cached rendition objects.

## 13.3 Region materialization

If dependency closure permits it, a decoder may materialize only:

- requested tile;
- requested crop;
- visible viewport;
- scanout band.

This could improve memory and latency in some applications.

It is an empirical performance question, not an assumed advantage.

---

# 14. EntropyFS as Procedural-State Persistence

## 14.1 Standalone VOLE remains primary

A `.vole` file must remain independently decodable.

EntropyFS is optional.

## 14.2 Natural object-store mapping

VOLE state naturally maps to immutable storage objects:

```text
visual object
procedural generator object
checkpoint object
transition chunk
residual object
palette
dictionary
entropy model
integrity object
edit graph
rendition state
```

The EntropyFS object graph may mirror the VOLE state graph without flattening the video into one opaque file blob.

## 14.3 Cross-video structural reuse

Exact immutable objects can be shared across:

- frames;
- clips;
- episodes;
- renditions;
- edits;
- projects;
- screen recordings;
- common graphics packages.

Identity must be exact or explicitly transformed plus residual.

“Looks the same” is not enough.

## 14.4 Physical accounting

For each stream and store, report:

```text
procedural descriptor bytes
transition bytes
checkpoint bytes
object bytes
residual bytes
entropy model bytes
dictionary bytes
integrity bytes
index bytes
shared-object physical bytes
attributed shared bytes
```

Do not report only descriptor size.

## 14.5 Garbage collection

A persistence layer traces reachable closure across:

- stream roots;
- checkpoints;
- transitions;
- objects;
- dictionaries;
- models;
- edits;
- renditions.

Reference cycles that affect materialization must be prohibited or otherwise explicitly bounded.

---

# 15. Equivalence-Preserving Procedural Re-Optimization

A crucial capability follows from making representation distinct from reconstruction.

Suppose foreground encoding produces state description \(D_0\).

Later, a deeper optimizer discovers \(D_1\) such that:

\[
\operatorname{Materialize}(D_0)
=
\operatorname{Materialize}(D_1)
\]

but:

\[
J(D_1)<J(D_0).
\]

Then the representation can be rewritten without changing the visual output.

Examples:

- replace repeated transitions with a parametric trajectory;
- merge identical objects;
- discover a longer-lived basis;
- convert many patches into a copy operation;
- replace local models with a shared model;
- compact checkpoint structure;
- discover affine state where block motion was used;
- reduce dependency depth;
- discover a reusable dictionary;
- replace a raster region with a procedural object plus smaller residual.

This is not a new lossy generation.

For a lossless source, pixels remain exact.

For an already lossy chosen reconstruction, the chosen reconstruction remains exact during representation-only optimization.

---

# 16. Non-Destructive Editing and Branching

A procedural state graph naturally supports derived timelines.

A new edit may reference immutable source states:

```text
BASE_STREAM
TRIM
TIME_MAP
OVERLAY
CROP
TRANSFORM
REPLACE_OBJECT
BRANCH
CONCAT
```

The edit graph may remain virtual until materialization.

This does not mean every edit is lossless. A crop or overlay changes the target. The important distinction is between:

- **representation-only change**, which preserves the target;
- **content edit**, which intentionally defines a new target;
- **lossy re-encoding**, which changes target samples through quantization.

VOLE should state the category explicitly.

---

# 17. Procedural Video Transport Scenarios

## 17.1 Desktop and terminal

Potential state:

```text
OBJECT editor_chrome
OBJECT font_atlas
OBJECT document_region

TRANSITION:
  COPY_RECT document_region dy=-18
  PATCH bottom_rows
  MOVE cursor
```

A conventional screen-content codec may already encode this well. The VOLE hypothesis is that persistent object identity and explicit transition state may provide additional storage, editing, caching, or transport advantages.

## 17.2 User-interface game

Persistent HUD, maps, panels, sprites, and palette objects may survive for long intervals.

The stream transmits transformations and mutations rather than repeatedly rediscovering every raster instance.

## 17.3 Static surveillance

A long-lived background may coexist with localized moving or sparse regions.

The correct measurement is whether persistent bases and local rebasing beat conventional inter coding after all metadata and search cost are counted.

## 17.4 Synthetic animation

A natively procedural source may have near-zero residual for large portions of the frame.

This is one of the strongest conceptual use cases because structure does not need to be inferred after rasterization.

## 17.5 Natural camera footage

Natural video is the hardest case.

The encoder may find useful:

- motion;
- affine/global transforms;
- persistent backgrounds;
- sparse changes;
- transform residuals.

But the residual may remain large.

VOLE must be willing to behave increasingly like a conventional codec where that is what the information demands.

## 17.6 Noise control

For synthetic random frames:

\[
|R|\approx|F|.
\]

Success means bounded overhead and graceful fallback, not surprising compression.

---

# 18. Materializer Architecture and Performance

## 18.1 Operation classes

A bounded materializer may execute:

```text
object fetch
checkpoint restore
transition apply
copy/blit
palette lookup
sparse scatter
motion compensation
affine transform
procedural generation
entropy decode
inverse transform
residual application
integrity verification
color conversion
```

## 18.2 Cheap state operations

Some procedural operations can be computationally cheap:

- exact reference;
- cached object reuse;
- fill;
- rectangle copy;
- palette lookup;
- sparse patches.

Other operations may be expensive:

- deep procedural generation;
- motion search at encode time;
- large affine resampling;
- inverse transforms;
- cold object fetches.

There is no universal speed claim.

## 18.3 Decoder versus encoder economics

The decoder may benefit from persistent state and direct operations.

The encoder initially risks being slower because it searches a broader explanation space.

This is why search-governance evidence is central.

## 18.4 Direct scanout hypothesis

A hardware or software implementation may materialize small regions just ahead of display scanout instead of allocating every full raster frame.

Whether this improves real systems depends on:

- display pipeline requirements;
- dependency locality;
- cache behavior;
- parallelism;
- residual access;
- synchronization.

This must be benchmarked.

---

# 19. Hardware Embodiments

A bounded VOLE materializer is amenable to CPU, SIMD, GPU, FPGA, or ASIC execution.

Potential fixed units include:

- object-cache engine;
- transition engine;
- rectangle copy engine;
- fixed-point transform engine;
- procedural primitive evaluator;
- motion/affine unit;
- sparse scatter unit;
- rANS decoder;
- palette unit;
- hash engine;
- residual combiner;
- color converter.

A future EPU or equivalent materializer should be designed from measured opcode distributions.

No clock rate, wattage, 4K/8K throughput, or silicon-area claim is established until implemented and measured.

---

# 20. Security and Bounded Execution

Procedural state creates additional attack surfaces compared with a simple raster decoder.

A conforming implementation must treat every stream as hostile.

Required bounds include:

- maximum dimensions;
- maximum sample count;
- maximum object count;
- maximum object size;
- maximum checkpoint size;
- maximum transition count per interval;
- maximum dependency depth;
- maximum generator state;
- maximum generator work;
- maximum copy area;
- maximum transform blocks;
- maximum rank bits;
- maximum entropy model;
- maximum dictionary;
- maximum residual expansion;
- maximum working memory.

Required validation includes:

- checked integer arithmetic;
- canonical lengths and varints;
- object existence;
- kind compatibility;
- cycle detection;
- self-reference rejection;
- rank range;
- copy bounds;
- overlap rules;
- entropy overread prevention;
- deterministic error handling;
- integrity verification.

The procedural language should remain non-Turing-complete.

---

# 21. Determinism

Normative materialization must define:

- integer widths;
- endianness;
- fixed-point formats;
- rounding;
- saturation/modular behavior;
- transform coefficients;
- interpolation;
- coordinate origin;
- clipping;
- object composition order;
- color conversion;
- rasterization rules;
- generator update order;
- entropy normalization;
- hash canonicalization.

Determinism and losslessness are distinct.

A deterministic perceptual profile can still be lossy.

A lossless profile must additionally preserve the declared source samples exactly.

---

# 22. Profiles

## 22.1 Lossless procedural profile

- exact raster target;
- deterministic state;
- exact candidate validation;
- no quantization loss;
- RAW fallback.

## 22.2 Native-procedural profile

- direct ingestion of authored state;
- optional canonical raster view;
- minimal residual where source is exactly representable.

## 22.3 Screen/remote profile

- persistent objects;
- drawing/copy operations;
- palette;
- sparse changes;
- low-latency checkpoints.

## 22.4 Archive profile

- stronger self-description;
- hashes;
- bounded access;
- corruption localization;
- object identity;
- exact replay;
- long-term universe versioning.

## 22.5 Scientific profile

- exact samples;
- acquisition metadata;
- deterministic replay;
- region integrity;
- explicit calibration associations.

## 22.6 Natural-video profile

- motion;
- affine/global bases;
- transforms;
- contextual entropy;
- procedural state only where it wins.

## 22.7 Perceptual profile

- explicit chosen reconstruction;
- rate-distortion objective;
- deterministic quantization;
- deterministic filtering;
- optional residual dropping under declared policy.

## 22.8 Low-latency stream profile

- shallow references;
- frequent checkpoints;
- bounded model updates;
- restricted expensive generators.

---

# 23. Empirical Research Program

The procedural-video thesis must be tested as several independent hypotheses rather than one large claim.

## 23.1 Phase 0 representation

A suitable first implementation:

```text
Gray8
lossless
fixed 64x64 tiles
bounded Procedural State Graph

State:
  immutable objects
  region instances
  checkpoint
  transition list

Representations:
  RAW
  FILL
  EXACT_REF
  UNCHANGED
  SPARSE_PATCH
  COPY_RECT
  INTEGER_TRANSLATION
  RANS_RESIDUAL
```

The important change from a conventional codec prototype is that **checkpoint + transition semantics are present from the beginning**.

## 23.2 Encoder strategies

Use the same reachable candidates with:

1. exhaustive search;
2. fixed deterministic heuristic;
3. DSFB-guided search.

## 23.3 Native procedural corpus

Create deterministic source-state sequences such as:

- moving vector rectangles;
- palette animation;
- scrolling text-like structures;
- parametric trajectories;
- deterministic procedural fields;
- object reuse;
- composition changes.

Compare:

- direct VOLE procedural ingestion;
- rasterize-then-encode VOLE;
- rasterize-then-encode conventional codecs.

This measures how much structure is lost by raster-first workflows.

## 23.4 Raster inverse-proceduralization corpus

Include:

- desktop;
- terminal;
- browser;
- game UI;
- animation;
- static surveillance;
- moving surveillance;
- microscopy/scientific;
- talking head;
- natural film;
- sports;
- camera pan;
- zoom;
- rotation;
- grain;
- random noise.

## 23.5 Conventional baselines

Depending on profile:

- FFV1;
- H.264 lossless;
- HEVC lossless;
- AV1 lossless;
- AV2 where tooling is available;
- VVC reference/lossless configurations;
- screen-content profiles;
- raw + generic compression controls.

All comparisons require equivalent raster input.

## 23.6 Procedural/scene baselines

For authored content, where practical also compare against:

- SVG/SMIL;
- Lottie;
- scene-graph formats;
- direct source project/state size;
- RDP-like drawing-order traces for synthetic screen tests.

These are not necessarily video codecs; they provide operational baselines for procedural representation.

## 23.7 Metrics

Measure:

```text
source raster bytes
source procedural bytes where applicable
complete VOLE bytes
checkpoint bytes
transition bytes
object bytes
residual bytes
model/dictionary bytes
integrity/index bytes
bits per pixel
bits per second
bits per state transition
encode CPU
decode CPU
materialization CPU
peak memory
full-frame latency
tile latency
scanline/region latency
seek latency
candidate evaluations
DSFB regret
object cache hit rate
dependency depth
residual fraction
procedural fraction
rebase count
```

## 23.8 Procedural fraction

Define an accounting metric carefully.

For example:

\[
P=
1-
\frac{B_{\mathrm{residual}}+B_{\mathrm{raw\ fallback}}}
     {B_{\mathrm{total}}}
\]

is one possible operational measure of how much encoded storage is not literal/residual raster payload.

It is **not** an entropy-theoretic measure and should not be called one.

Alternative metrics should be reported if they better explain the state.

## 23.9 Innovation-rate curve

Measure transmitted bytes against structural changes.

For a controlled scene with long static periods and discrete transitions, plot:

\[
\text{bytes per interval}
\]

against:

\[
\text{declared structural changes per interval}.
\]

The goal is to determine whether bandwidth follows state innovation on suitable content.

---

# 24. Ablation Program

A cumulative ladder may be:

```text
P0   RAW
P1   + FILL
P2   + persistent exact objects
P3   + checkpoints/transitions
P4   + unchanged state
P5   + sparse patches
P6   + COPY_RECT
P7   + integer translation
P8   + contextual entropy
P9   + DSFB search governor
P10  + palettes
P11  + parametric trajectories
P12  + affine/global state
P13  + transform residual
P14  + generated bases
P15  + shared dictionaries
P16  + EntropyFS cross-video persistence
```

Pair this with leave-one-out experiments.

A mechanism is not credited merely because the cumulative result improved after it was added.

---

# 25. Falsification Criteria

The following outcomes should count against the architecture in the tested domain.

1. Procedural state plus residual is consistently larger than strong codec baselines after complete accounting.
2. Transition/checkpoint overhead erases savings.
3. Persistent identity offers negligible reuse on realistic corpora.
4. Inverse proceduralization search cost is impractical.
5. DSFB does not beat simple deterministic heuristics on search economics.
6. Residuals remain near raster size for most claimed target domains.
7. Object-store locality makes decode materially slower without compensating benefit.
8. Checkpoint frequency required for resilience destroys density.
9. Parametric trajectories rarely replace enough per-frame state to matter.
10. Procedural generators almost always lose after residual correction.
11. Region materialization provides no practical latency or memory improvement.
12. Cross-video exact sharing is too rare to matter.
13. State-graph complexity creates unacceptable security or conformance burden.
14. A conventional codec plus ordinary dedup achieves the same outcomes more simply.
15. Hardware execution is less efficient than conventional decode without a compensating systems advantage.

Negative results remain part of the evidence record.

---

# 26. Research Hypotheses

## H1 — Procedural-state density

On at least some useful video classes, persistent procedural state plus residual can represent the same lossless raster output in fewer total bytes than a restricted raster-centered representation.

## H2 — Native-procedural preservation

For content that originates as structured graphics or simulation state, preserving procedural state before rasterization can reduce storage or transport cost relative to rasterize-then-code workflows.

## H3 — Inverse proceduralization

For at least some raster-origin video, bounded structural search can discover persistent state whose description plus exact residual is cheaper than treating every interval primarily as a new raster coding problem.

## H4 — Structural-innovation transport

On state-stable video, transmitted bytes can track structural innovation more closely than raster resolution × frame rate, while degrading gracefully toward raster coding as residual information increases.

## H5 — Persistent identity

Maintaining exact object identity across time can reduce repeated representation work and improve cache reuse, storage sharing, or editing operations.

## H6 — DSFB search efficiency

Residual-governed DSFB search can approach exhaustive procedural-hypothesis quality with fewer candidate evaluations than exhaustive search and can outperform fixed heuristics under regime change.

## H7 — Local slew and rebase

Localized residual slew can identify where a procedural explanation has become stale, allowing regional rebaselining without resetting unaffected state.

## H8 — Partial materialization

Region-, tile-, or scanout-oriented materialization can reduce memory traffic or latency for some playback and remote-display workloads.

## H9 — Hybrid resolution independence

Procedural portions of a stream can be materialized at multiple resolutions while raster residual components remain explicitly resolution bound.

## H10 — Cross-video entropy-native persistence

An EntropyFS-backed VOLE repository can produce measurable physical savings through exact reuse of immutable procedural objects, models, dictionaries, checkpoints, or residual structures across multiple videos.

## H11 — Equivalence-preserving optimization

Background optimization can discover lower-cost procedural representations that materialize exactly the same reconstruction.

## H12 — Hardware materialization

A bounded procedural video ISA can be accelerated effectively by SIMD/GPU/FPGA/ASIC execution after the actual operation mix is measured.

Each hypothesis has an empirical rejection path.

---

# 27. Prior-Art Mapping by Mechanism

| Disclosed mechanism | Strong antecedent | Position in this disclosure |
|---|---|---|
| Motion compensation | H.26x, MPEG, AV1, VVC | Candidate state transition; not claimed alone |
| Transform coding | JPEG/MPEG/H.26x/AV1/VVC | Residual family; not claimed alone |
| Scene graph | VRML, X3D, MPEG-4 BIFS | Strong antecedent; VOLE adds raster inverse proceduralization + residual |
| Streamed scene mutation | MPEG-4 BIFS-Command/BIFS-Anim | Strong antecedent to state-transition streaming |
| Object-based audiovisual coding | MPEG-4 Visual | Strong antecedent to persistent visual objects |
| Vector/declarative animation | SVG/SMIL, Lottie, Rive | Strong antecedent to procedural temporal graphics |
| Procedural synthesis | Perlin and broad procedural graphics | Generator candidate; no novelty claimed |
| Model-based video | model-based/analysis-by-synthesis coding | Strong antecedent to parameterized reconstruction |
| Dynamic temporal models | dynamic-texture research | Antecedent to generative temporal state |
| Remote drawing commands | RDP graphics orders and caches | Strong antecedent to procedural screen transport |
| COPY/ADD/RUN delta | VCDIFF | Generalized into spatial/video transition language |
| Exact content addressing | Venti and later CAS | Applied to video state objects and dependencies |
| Screen block copy/palette | HEVC SCC, AV1 screen tools | Included as bounded state/region operations |
| Scalable/derived media | SVC, ISO derived visual tracks | Related to views, renditions, transformation graphs |
| Reconfigurable decode graphs | MPEG RVC | Related; VOLE remains bounded/non-Turing-complete |
| Neural scene/video models | learned implicit video/scene compression | Related alternative; VOLE is explicitly non-ML |
| Residual-guided search | broad codec heuristics + DSFB | DSFB embodiment uses residual drift/slew with zero authority |
| Representation repacking | transcoding/repacking systems | VOLE emphasizes same-reconstruction procedural rewriting |
| Cross-video sharing | storage dedup | Integrated with normative procedural object identity |

The paper does not depend on claiming novelty for any row in isolation.

---

# 28. Broad Embodiments

The disclosed architecture explicitly includes the following embodiments.

## 28.1 Standalone `.vole` codec

A portable file contains objects, checkpoints, transitions, residuals, models, dictionaries, and integrity metadata.

## 28.2 Native procedural capture

A renderer, UI toolkit, game engine, or simulation emits VOLE state directly before rasterization.

## 28.3 Raster inverse-procedural encoder

A conventional raster input is analyzed into bounded procedural hypotheses plus residual.

## 28.4 Procedural streaming transport

A receiver maintains replicated state from OBJECT/CHECKPOINT/TRANSITION/RESIDUAL packets.

## 28.5 Remote desktop transport

VOLE functions as a generalization of drawing-order remoting with raster fallback.

## 28.6 Archive codec

State, checkpoints, and exact reconstruction are stored with strong integrity and bounded long-term decode semantics.

## 28.7 Scientific recording

Exact raster observations coexist with procedural bases and acquisition metadata.

## 28.8 Content-addressed video repository

Many videos share immutable visual/procedural objects.

## 28.9 Non-destructive editor

Edits are state-graph branches over immutable source objects.

## 28.10 Resolution-flexible procedural media

Procedural components are materialized at target view resolution while resolution-bound residuals are handled explicitly.

## 28.11 On-demand tile materializer

Only requested regions are rasterized.

## 28.12 Direct scanout materializer

A decoder materializes bounded output bands near display consumption rather than requiring full-frame raster residency where system constraints permit.

## 28.13 GPU materializer

Descriptor classes are batched into deterministic kernels.

## 28.14 FPGA/ASIC materializer

Fixed units implement the measured procedural ISA.

## 28.15 EntropyFS-native storage

VOLE objects map directly into an entropy-native content-addressed graph.

## 28.16 Offline optimizer

Existing video is structurally re-analyzed and rewritten without changing reconstruction.

## 28.17 Forensic/evidence mode

Every representation decision may emit a receipt containing:

- input hash;
- candidate hypotheses;
- candidate costs;
- chosen state transition;
- residual cost;
- reconstruction hash;
- DSFB diagnostics;
- encoder version;
- environment.

---

# 29. Implementation Sequence

A disciplined implementation should evolve the procedural ontology early rather than retrofit it onto a conventional frame codec.

## Phase A — bounded procedural core

- one native-Rust crate;
- Gray8;
- `.vole` stream;
- object table;
- checkpoint;
- transition list;
- RAW;
- FILL;
- exact frame output;
- hostile-input limits;
- canonical serialization.

## Phase B — persistent objects

- content hashes;
- exact object references;
- instance identity;
- unchanged-state transitions;
- sparse patches.

## Phase C — 2D state transitions

- COPY_RECT;
- MOVE_RECT;
- integer translation;
- bounded region composition.

## Phase D — entropy floor

- native deterministic rANS or equivalent;
- explicit model accounting;
- RAW fallback.

## Phase E — exhaustive inverse proceduralization court

- candidate generators;
- exact materialization validation;
- complete cost accounting;
- exhaustive oracle.

## Phase F — DSFB search

- fixed heuristic;
- DSFB observer;
- deterministic sentinels;
- local rebase;
- regret receipts.

## Phase G — parametric dynamics

- bounded trajectories;
- velocity/acceleration;
- fixed-point spline or equivalent;
- exact transition collapse.

## Phase H — palette and screen content

- palette objects;
- palette transitions;
- screen corpus;
- remote-display corpus.

## Phase I — affine/global state

- fixed-point affine;
- global transforms;
- region splitting and rebasing.

## Phase J — transform residual floor

- integer transform;
- contextual entropy;
- natural-video corpus.

## Phase K — generated bases

- small deterministic generator ISA;
- exact residual correction;
- mandatory negative controls.

## Phase L — EntropyFS persistence

- external object-store adapter;
- shared objects;
- store accounting;
- GC/reference closure.

## Phase M — offline re-optimization

- representation-equivalence proof by reconstruction hash;
- deeper procedural search;
- transactionally replace cheaper state.

## Phase N — native procedural ingest

- direct source-state API;
- compare against raster-first ingest.

## Phase O — transport

- OBJECT;
- CHECKPOINT;
- TRANSITION;
- RESIDUAL;
- INTEGRITY;
- loss/restart courts.

## Phase P — partial materialization

- tile/crop decode;
- cache policy;
- memory/latency courts.

## Phase Q — perceptual profile

Only after exact architecture is stable.

---

# 30. Why Non-ML Is a Deliberate Property

VOLE does not require:

- training data;
- learned neural weights;
- opaque latent tensors;
- neural inference runtime;
- probabilistic generation at decode time.

It may use:

- deterministic hashes;
- exact matching;
- motion search;
- affine search;
- combinatorial coding;
- explicit dictionaries;
- explicit entropy models;
- finite procedural generators;
- fixed-point trajectories;
- residual analysis;
- DSFB observers;
- exhaustive or bounded search.

The absence of ML is not claimed to guarantee superior compression.

It enables different engineering properties:

- deterministic replay;
- clear decoder specification;
- bounded execution;
- inspectable state;
- explicit residual accountability;
- independent conformance;
- no model distribution dependency;
- exact candidate verification.

---

# 31. Limitations and Open Questions

## 31.1 Natural video may remain raster-dominant

Camera video may contain too much non-procedural detail for a compact state graph to dominate.

## 31.2 Procedural inference may be expensive

Inverse proceduralization expands encoder search dramatically.

## 31.3 Metadata can erase gains

Fine-grained state may cost more than it saves.

## 31.4 State locality matters

Content-addressed references can create cold-cache I/O.

## 31.5 Checkpoints cost bytes

Robust streaming requires bounded restart state.

## 31.6 Persistent identity can be fragile

Small pixel differences destroy exact object sharing unless a transformed-base-plus-residual candidate remains economical.

## 31.7 Procedural generators can overfit

A complicated generator that merely relocates information from residual to parameters is not a win.

## 31.8 Resolution independence is partial

Raster residuals remain tied to sampling semantics.

## 31.9 Conventional codecs are extraordinarily strong

VOLE must compete against decades of optimization, especially on natural video.

## 31.10 Scene-description prior art is substantial

MPEG-4 BIFS, X3D, vector animation, and remote rendering already prove much of the broad procedural-media concept. VOLE's research value therefore depends on the measurable utility of the full architecture—particularly inverse proceduralization, exact residual closure, procedural/raster hybridization, residual-governed search, and entropy-native persistence—not on pretending scene graphs are new.

---

# 32. Conclusion

VOLE is proposed here not merely as another raster video codec but as a **procedural video storage, transport, and materialization system**.

Its defining model is:

\[
G_{t+1}=\Phi(U,G_t,\Delta_t),
\]

\[
F_t=M(U,G_t,V)\oplus_{\rho}R_t.
\]

The persistent object is the bounded deterministic state and its evolution. The raster frame is a materialized view.

This architecture permits two fundamentally different source paths:

\[
\text{native procedural source}
\rightarrow
\text{VOLE state}
\]

and:

\[
\text{raster source}
\rightarrow
\text{inverse proceduralization}
\rightarrow
\text{VOLE state}+\text{residual}.
\]

The central engineering objective is not to deny entropy but to move repeated explainable structure out of repeated raster payload:

\[
\boxed{
\text{store the deterministic explanation}
+
\text{store the information that explanation cannot reproduce}
}
\]

If the explanation is compact, storage and transport may fall sharply.

If the explanation is poor, the residual grows and conventional coding should win.

That graceful failure behavior is essential.

The architecture deliberately acknowledges extensive prior art in hybrid video coding, MPEG-4 object and scene description, BIFS scene updates, VRML/X3D, declarative/vector animation, procedural graphics, model-based coding, dynamic textures, remote desktop drawing orders, delta compression, content-addressed storage, scalable media, and derived-media systems. No broad claim of inventing procedural graphics or scene streaming is made.

The disclosed design instead places these ideas into one deterministic video system with a particular set of constraints and interactions:

- raster frames are materializations rather than mandatory primary stored objects;
- procedural state persists across time;
- arbitrary raster sources can enter through inverse proceduralization;
- exact residuals close the gap between model and observation;
- a universal raster fallback preserves generality;
- procedural state can be streamed as checkpoints and transitions;
- immutable objects can be shared within and across videos;
- DSFB observes residual trajectories to govern search without holding authority;
- equivalent procedural representations can be optimized later without changing reconstruction;
- EntropyFS can persist the state graph directly;
- region- and view-specific materialization can avoid unnecessary raster work where practical;
- every claimed advantage remains subject to reproducible empirical evidence.

The most important research question is therefore not:

> Can video be replaced by mathematics?

All digital codecs already rely on mathematics, and arbitrary information cannot be made to disappear.

The stronger and testable question is:

> **How much of a video's raster sequence can be replaced by compact, persistent, deterministic generative state before the residual becomes the dominant representation, and can that state be stored, transported, searched, materialized, and re-optimized more efficiently than raster-centered alternatives?**

VOLE exists to answer that question experimentally.

---

# References

1. ITU-T. **Recommendation H.264: Advanced video coding for generic audiovisual services.** ITU-T H.264 / ISO/IEC 14496-10.
2. Bross, B.; Chen, J.; Ohm, J.-R.; Sullivan, G. J.; Wang, Y.-K. **Developments in International Video Coding Standardization After AVC, With an Overview of Versatile Video Coding (VVC).** *Proceedings of the IEEE*, 109(9), 2021.
3. Bross, B.; Wang, Y.-K.; Ye, Y.; Liu, S.; Chen, J.; Sullivan, G. J.; Ohm, J.-R. **Overview of the Versatile Video Coding (VVC) Standard and its Applications.** *IEEE Transactions on Circuits and Systems for Video Technology*, 31(10), 2021. DOI: 10.1109/TCSVT.2021.3101953.
4. Alliance for Open Media. **AV1 Bitstream & Decoding Process Specification.**
5. Alliance for Open Media. **AV2 Bitstream & Decoding Process Specification.** 2026.
6. Xu, J.; Joshi, R.; Cohen, R. A. **Overview of the Emerging HEVC Screen Content Coding Extension.** *IEEE Transactions on Circuits and Systems for Video Technology*, 26(1), 2016. DOI: 10.1109/TCSVT.2015.2478706.
7. ISO/IEC. **ISO/IEC 14496-2: Coding of audio-visual objects — Visual.** MPEG-4 Visual.
8. ISO/IEC. **ISO/IEC 14496-11: Coding of audio-visual objects — Scene description and application engine.** MPEG-4 BIFS/XMT/MPEG-J.
9. MPEG. **Scene Description and Application Engine.** Technical overview of MPEG-4 BIFS.
10. MPEG. **What is BIFS?** MPEG-4 Scene Description and Application Engine FAQ/technical description.
11. Signès, J.; Fisher, Y.; Eleftheriadis, A. **MPEG-4's Binary Format for Scene Description.** *Signal Processing: Image Communication*, 15, 2000.
12. Web3D Consortium / ISO/IEC. **X3D — Extensible 3D Graphics, ISO/IEC 19775 family.** Current X3D standards and architecture.
13. W3C. **SVG Animations / SVG and SMIL animation specifications.**
14. Lottie Animation Community. **Lottie Animation Format Specification.**
15. Rive. **State Machine and Runtime Documentation.**
16. Perlin, K. **An Image Synthesizer.** SIGGRAPH 1985, pp. 287–296. DOI: 10.1145/325334.325247.
17. Parish, Y. I. H.; Müller, P. **Procedural Modeling of Cities.** SIGGRAPH 2001, pp. 301–308. DOI: 10.1145/383259.383292.
18. Aizawa, K.; Huang, T. S. **Model-Based Image Coding: Advanced Video Coding Techniques for Very Low Bit-Rate Applications.** *Proceedings of the IEEE*, 83(2), 1995, 259–271. DOI: 10.1109/5.364463.
19. Doretto, G.; Chiuso, A.; Wu, Y. N.; Soatto, S. **Dynamic Textures.** *International Journal of Computer Vision*, 51(2), 2003, 91–109.
20. Microsoft. **Remote Desktop Protocol: Graphics Device Interface Acceleration Extensions — Primary Drawing Orders.** Microsoft Open Specifications.
21. Microsoft. **Remote Desktop Protocol: Basic Connectivity and Graphics Remoting — Graphics Update structures.** Microsoft Open Specifications.
22. Korn, D.; MacDonald, J.; Mogul, J.; Vo, K.-P. **The VCDIFF Generic Differencing and Compression Data Format.** RFC 3284, IETF, 2002.
23. Quinlan, S.; Dorward, S. **Venti: A New Approach to Archival Data Storage.** FAST 2002, USENIX Association.
24. Niedermayer, M.; Rice, D.; Martinez, J. **FFV1 Video Coding Format Versions 0, 1, and 3.** RFC 9043, IETF, 2021.
25. Schwarz, H.; Marpe, D.; Wiegand, T. **Overview of the Scalable Video Coding Extension of the H.264/AVC Standard.** *IEEE Transactions on Circuits and Systems for Video Technology*, 17(9), 2007.
26. ISO/IEC. **ISO/IEC 14496-12: ISO Base Media File Format.**
27. ISO/IEC. **ISO/IEC 23001-16:2021 — Derived visual tracks in the ISO base media file format.**
28. ISO/IEC. **ISO/IEC 23001-4:2017 — Codec configuration representation / Reconfigurable Video Coding framework.**
29. Tang, L.; Zhang, X.; Zhang, G.; Ma, X. **Scene Matters: Model-based Deep Video Compression.** ICCV 2023, pp. 12481–12491. Cited as a learned scene-level video representation antecedent and contrast; VOLE is non-ML.
30. de Beer, R. **EntropyFS: Entropy-Native Configurational Storage as a Filesystem Substrate — Broad Prior-Art Technical Disclosure and Research Architecture.** Zenodo, 2026. DOI: 10.5281/zenodo.22092869.
31. de Beer, R. **Drift–Slew Fusion Bootstrap: A Deterministic Residual-Based State Correction Framework.** Zenodo, 2026. DOI: 10.5281/zenodo.18706455.
32. de Beer, R. **DSFB and Computer Graphics: Deterministic Structural Semiotics.** Zenodo, 2026. DOI: 10.5281/zenodo.19432403.
33. de Beer, R. **DSFB-GPU — Clear-Box Pure Deterministic Inference CUDA Acceleration for Replayable Trace-Event Verdicts.** Zenodo, 2026. DOI: 10.5281/zenodo.20346478.

---

## Disclosure Note

This document is intended as a technical publication and prior-art disclosure. It is not a legal opinion regarding patentability, validity, infringement, ownership, or freedom to operate. Statements concerning technical antecedents are engineering comparisons, not legal claim construction. Formal patent analysis, if required, should be performed separately against the complete claims, specifications, prosecution histories, and relevant jurisdictions.
