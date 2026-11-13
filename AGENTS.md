# AGENTS.md — Hybrid 2D/3D Rust Game

> Repository-level instructions for AI coding agents. Read this file before making changes.

## 1. Project mission and scope

Build **one complete, playable hybrid 2D/3D game in Rust** with a **game-specific internal engine**.
The engine is an implementation detail of the game, **not** a separate product, editor, SDK, general-purpose framework, or Unity/Unreal replacement.

The game must be designed to run on **Windows and Linux**. Work toward an enjoyable, functioning game, not an ever-growing collection of engine abstractions.

**Priorities, in order:**
1. A working game and playable vertical slices.
2. Correct gameplay and responsive controls.
3. Visual coherence, smooth UI interactions, and robust animation.
4. Maintainable, testable Rust code.
5. Profile-guided performance and reliable builds.

**Do not invent game genre, story, rules, art direction, camera type, multiplayer, save format, or monetization.** The project's current code, assets, and explicit user requirements are authoritative. If a gameplay requirement is unspecified, implement only the infrastructure required by the active task, making small reversible assumptions and documenting them.

## 2. Canonical technology stack

**Required foundation:**
- **Rust stable**, **Cargo** — language and build system.
- **winit** — native window/event lifecycle and input events.
- **wgpu** — GPU rendering for 2D, 3D, and game UI.
- **WGSL** — GPU shaders and material effects.
- **glam** — vectors, matrices, quaternions, camera math.
- **serde** — serialization and configuration.
- **image** — image decoding, especially PNG and WebP.
- **tracing** — diagnostics and instrumentation.

**Adopt only when an actual feature needs it:**
- `bevy_ecs` for complex entity/component/system needs; otherwise use clear Rust structs and modules.
- `taffy` for responsive Flexbox/Grid-like UI layout.
- `glyphon` / `cosmic-text` for GPU text rendering and Unicode shaping.
- `gltf` for glTF/GLB models, transforms, and animation data.
- `rapier2d` / `rapier3d` for 2D/3D physics when necessary (do not add both by default).
- `kira` for audio and music.
- `gilrs` for gamepad support.
- `rfd` for developer file dialogs if specifically needed.
- `egui` + `egui-wgpu` **only for optional development/debug UI** (FPS panel, logs, inspectors, diagnostics), not as the shipped fantasy game UI.
- `thiserror` / `anyhow`, `rayon`, `tokio`, profiling tools, etc. only for justified needs.

**Do not add by default:** Tauri, Angular, React, Vue, Svelte, HTML/CSS/WebView UI, a standalone editor, node-based material editor, generalized plugin platform, visual scripting language, scripting runtime, or multiple renderer backends.

Inspect the repository's existing versions and APIs before coding. Choose **mutually compatible crate versions**, commit the `Cargo.lock` for this application, and never fabricate APIs or dependency compatibility.

## 3. Architectural boundaries

Start with **one Cargo application**, separated into logical modules. Create workspace crates only when a measured build-time, testing, or dependency-boundary benefit justifies it.

Suggested layout (create modules as needed; do not create empty files):

```text
Cargo.toml
Cargo.lock
src/
  main.rs
  app.rs                      # lifecycle, main loop, error handling
  engine/                     # internal subsystems for THIS game
    render/                   # device/surface, 2D/3D, render targets, post FX
    materials/                # material definitions and instances
    assets/                   # loading, cache, handles, validation
    scene/                    # transforms, cameras, world representation
    animation/                # tweens, timelines, skeletal/sprite if needed
    ui/                       # custom game UI, input, text, compositor
    input/                    # action mappings and input state
    audio/                    # when needed
    physics/                  # when needed
  game/
    states/                   # main menu, gameplay, pause, settings
    gameplay/                 # gameplay-specific entities and rules
    menus/                    # concrete game menus/HUD
    settings/                 # persisted user settings
    save/                     # save/load when needed
assets/
  ui/
  textures/
  models/
  audio/
  fonts/
shaders/
tests/
```

Rules:
- `game/` may depend on `engine/`; `engine/` **must not depend on** game-specific systems.
- Keep game state and CPU-side simulation separate from render extraction and GPU submission.
- Share **one wgpu device/queue/resource infrastructure** across 2D, 3D, and UI.
- Use stable IDs/handles for assets; avoid duplicated textures and shader pipelines.
- Keep module interfaces narrow; avoid global mutable state and circular dependencies.
- Do not write a general-purpose framework when a targeted implementation solves the task.

## 4. Main loop and game lifecycle

The application must handle creation, window events, resizing, DPI changes, focus loss, minimized state, and clean shutdown.

Recommended logical update order:
1. Collect `winit` events and update input snapshots.
2. Route pointer/keyboard events to the topmost eligible UI element; consume UI events before world actions when appropriate.
3. Apply game state transitions and gameplay input.
4. Run deterministic fixed-step updates for physics/simulation as needed.
5. Update variable-time animation and UI state using clamped frame delta.
6. Recalculate dirty layouts/transforms and extract visible render data.
7. Upload changed GPU resources; encode render passes; submit and present.

Use a **fixed timestep** for simulation that needs determinism and use interpolation for presentation if appropriate. Animation must not depend on frame rate. Avoid spiral-of-death loops after long pauses; cap fixed updates per frame.

Game state transitions must be explicit (e.g., `MainMenu`, `Loading`, `Playing`, `Paused`, `Settings`, `Exiting`), without implementing unused screens. Input must not leak from a focused menu into gameplay.

## 5. Hybrid 2D/3D scene and rendering

- Both 2D and 3D use the **same renderer infrastructure**, asset manager, window, and frame lifecycle.
- Provide perspective and orthographic camera support when the game needs them.
- Keep distinct coordinate spaces explicit: world, view, clip/NDC, physical pixels, UI logical points.
- Define layering and depth rules: 3D meshes, world-space 2D sprites/effects, screen-space HUD/UI.
- World-space 2D must follow appropriate camera/depth behavior; screen-space UI must not accidentally inherit 3D depth.
- Use an explicit render sequence, initially simple: 3D scene → applicable 2D world layers → scene postprocessing → game UI → presentation.
- Revisit pass order when HDR effects, transparent objects, bloom, or postprocessing demand a different pipeline.
- Handle alpha blending and color spaces correctly (`sRGB` inputs vs linear shader math, premultiplied vs straight alpha).
- Reuse render targets/buffers where safe. Add shadows, PBR, compute passes, instancing, render graphs, or extensive postprocessing only for required visuals.
- Never assume `wgpu` features/limits are supported; query adapter capabilities and define fallback behavior.

## 6. Materials, shaders, and texture pipeline

Design materials as:
- **Material definition:** WGSL shader, bindings, textures, sampler/blend/depth rules.
- **Material instance:** per-object shader parameters (hover, glow, opacity, colors, UV offset, etc.).

Requirements:
- A change to one material instance must not unintentionally change all others.
- Cache shader modules, pipelines, textures, and bindings; **never recompile shaders each frame**.
- Decode PNG/WebP when importing/loading. WebP Lossless is suitable for many UI assets, including alpha.
- Preserve relevant alpha/mask channels. Treat **mask data as linear**, not color-managed sRGB data.
- Disk compression (WebP/AVIF) is distinct from GPU compression (e.g. BCn/KTX2). Do not promise GPU memory savings just from using WebP.
- Use correct asset loading, fallback/error diagnostics, and handle lifetime; never embed user asset paths as unvalidated assumptions.
- Where appropriate, use 9-slice rendering so decorative button corners and crystals are not distorted.
- True HDR bloom and a localized painted glow mask are different techniques; implement the cheapest technique sufficient for the actual art direction.

## 7. Custom shipped Game UI (important)

The **shipped game UI is custom**, rendered through `wgpu`, not a browser or egui widget tree. It must support the project's stylized fantasy buttons and shader-driven animation.

Core capabilities as requirements arise:
- UI tree and stable node IDs; panel, image, button, text, list/scroll containers.
- Layout (simple positioning initially; Taffy for complex responsive layouts).
- DPI scaling, clipping, z-order, focus, click/drag/hover routing, keyboard navigation as needed.
- GPU text through glyphon/cosmic-text where needed; keep editable button labels separate from artwork.
- Batched UI draw operations while preserving compositing order.

### Fantasy textured button

The user has a **stone/fantasy button asset** with decorative crystals and a **separate grayscale glow mask**. Preserve and reuse the original art when present; never overwrite it with placeholders.

The button should support:
- States: `Normal`, `Hovered`, `Pressed`, `Disabled`, optionally `Selected`.
- Base texture + glow mask + WGSL material, with per-button `hover_amount`, `glow_intensity`, `glow_color`, `opacity`.
- Smooth hover transitions, including correct interruption and reversal on fast cursor movement.
- Stone visuals may remain unchanged while crystal glow/brightness increases.
- Text is rendered separately unless the existing asset requires otherwise.

### Pixel-perfect hit testing

Provide configurable modes:

```rust
// Conceptual API; adapt to the project's actual types.
enum HitTestMode {
    Rectangle,
    Alpha { threshold: f32 },
    CustomMask { mask: TextureId, threshold: f32 },
}
```

- Input should respond to the **visible shape** of the button, not its transparent rectangular texture boundary.
- Convert pointer location via layout/inverse transforms and UV mapping before testing.
- Store a CPU-side alpha or hit mask for lightweight sampling; **no per-pointer GPU readback**.
- Respect clipping, stacking order, disabled elements, pointer capture, and 9-slice mapping.
- Transparent pixels of a top button must allow input to reach eligible elements behind it.
- The **glow mask does not automatically define the hit region**; emissive halos should not expand clickable areas by accident.

### Group opacity and masked materials

This must **not** repeat the Unreal UMG behavior where masked-material buttons abruptly pop from invisible to fully visible during parent fade-in.

- Support `Masked`/alpha-cutout material behavior for the button itself.
- Support smooth **parent/group opacity** independently of internal masking.
- When required for correct overlapping compositing, render child content to an **offscreen transparent texture**, then apply opacity to the completed group at composition time.
- Use consistent alpha blending conventions and account for nested fades, clipping, and intermediate texture reuse.
- Do not silently change masked materials to translucent materials.
- Avoid allocating an offscreen texture for every widget; use it only when correct group semantics need it.

## 8. Animation systems

Implement only the animation types needed by the game, using a consistent time/update model:
- Tween/easing for common UI transitions.
- Keyframes and timelines for coordinated animations.
- Sprite-sheet/UV animations for 2D effects if required.
- Transform/material-parameter tracks.
- Skeletal animation for animated 3D models when actually used.

Requirements:
- Independent tracks must coexist without overwriting unrelated parameters.
- Support interruption, reversal, and transition from **current visual state**.
- Animations must not restart unexpectedly due to UI rebuild, state change, or temporary frame drops.
- Distinguish fixed-step gameplay simulation from frame-based visual interpolation.
- Ensure `Masked` material animations and parent opacity can coexist.

## 9. Gameplay, scenes, and camera

- The **game rules are game-specific**. Read current design notes/code before inventing mechanics.
- Organize game logic into small testable systems: player/control logic, interactions, world state, UI reactions, transitions.
- Introduce ECS only when complexity merits it; `bevy_ecs` is acceptable without adopting the Bevy renderer.
- Maintain clear scene ownership, loading/unloading, and transitions; avoid resource leaks on scene changes.
- Support both 2D and 3D transforms/cameras through explicit conventions instead of guessing spatial units.
- Implement physics, pathfinding, AI, procedural generation, networking, or multiplayer **only** upon actual game requirements.
- User settings such as graphics, sound, and controls must have one source of truth and be propagated to dependent systems.

## 10. Audio, persistence, and assets

- Introduce audio playback, effects, volume groups, and music transitions when gameplay needs them; `kira` is preferred if compatible.
- Centralize asset loading and handle missing/corrupt files with useful errors and fallbacks.
- Favor glTF/GLB for 3D models and WebP/PNG for UI textures; don't prevent other formats when justified.
- Provide deterministic asset IDs/paths and avoid duplicated GPU uploads.
- For save/load, use versioned serializable data and **atomic writes** where practical; never save raw GPU handles.
- Preserve backward compatibility for existing saves whenever possible; otherwise document migration.
- Distinguish shipped assets from developer-only examples and temporary placeholders.

## 11. Coding quality and safety

- Use idiomatic stable Rust with clear ownership and meaningful errors.
- Prefer `Result` to panics at I/O, GPU, decoding, and OS boundaries.
- Avoid `unwrap()`/`expect()` for routine runtime failures.
- Avoid `unsafe` unless necessary, documented, and reviewed.
- Do not introduce unnecessary abstraction layers, dependencies, macros, or reflection.
- Put comments on invariants, coordinate conversions, compositing, lifetimes, and non-obvious decisions.
- Do not implement features using mocks or stubs while claiming they work.
- Never delete or overwrite existing user assets without explicit instruction.
- English identifiers and code documentation are preferred; explain decisions and progress **in Ukrainian**.

## 12. Performance and diagnostics

Performance targets (60/120/144 FPS) are **goals, not guarantees**; hardware and scene complexity determine achieved FPS.

- Record CPU frame time, GPU timing when available, draw calls, allocation counts, and memory use before optimization.
- Avoid allocations/uploads/pipeline creation in hot loops when practical.
- Batch compatible sprites and UI elements without breaking layering or alpha composition.
- Update dirty transforms/layout/material parameters rather than rebuilding everything needlessly.
- Avoid blocking file I/O on critical rendering paths.
- Use `tracing` and available CPU/GPU profilers; no invented benchmarks or claims.
- Optimize measured bottlenecks, not speculation.

## 13. Developer tools, inspection, and debug UI

Use established tools instead of building a general-purpose editor for this game:

| Purpose | Preferred tool or approach |
| --- | --- |
| Source / configuration editing | VS Code with `rust-analyzer`; any editor with equivalent support is acceptable. |
| Rust build and dependency management | `cargo`, `rustup`, committed `Cargo.lock`. |
| Format and static analysis | `rustfmt`, `clippy`, `cargo check`. |
| Tests | Rust unit/integration tests; focused visual/manual acceptance tests. |
| Repository navigation | `rg` (ripgrep), `git grep`, `find`, and targeted file reading. |
| Change review | `git status`, `git diff`, `git diff --check`. |
| Rust diagnostics / instrumentation | `tracing`; profilers such as Tracy where justified. |
| GPU capture | RenderDoc on supported backends; consult platform support first. |
| WGSL validation | `naga` parser/validator, plus actual wgpu pipeline creation. |
| 2D art and masks | Krita, GIMP, or the project's existing art pipeline. |
| 3D art | Blender and glTF/GLB validation. |
| Asset file watching | `notify` in **development builds only**, when hot reload is implemented. |

**egui role:** `egui` + `egui-wgpu` is permitted for **developer/debug overlays**, not as the shipped stylized Game UI. An optional debug overlay may offer FPS/frame timings, device/backend info, scene stats, texture/pipeline status, asset reload errors, and live sliders for UI glow/opacity/hover duration. Keep it behind a feature or development-only configuration. Disable or omit developer controls in release builds unless explicitly requested.

Keep debug controls wired to the actual running game/material state, with a clear distinction between temporary runtime overrides and persistent edits. Do not silently write assets/configuration because a debug slider moved.

## 14. File inspection, editing, and change safety

Before editing files:

1. Inspect `git status --short` to identify user or other-agent modifications. **Do not discard them.**
2. Read `AGENTS.md`, the relevant `Cargo.toml`, source modules, tests, shader files, and any affected configuration.
3. Search for definitions and uses with `rg`/`git grep` before changing public types, resources, paths, shaders, or bindings.
4. Confirm who owns each asset and how it is loaded. Respect existing conventions and imported art.
5. Choose the smallest change set that completes the requested feature. Avoid unrelated formatting or refactoring.

While editing:

- Make focused, inspectable patches using the available file-editing capabilities; never fabricate a patch result.
- Preserve formatting and module boundaries. Keep code and asset names consistent.
- Do not overwrite or replace source artwork, animation data, masks, 3D models, fonts, or saved games without explicit authorization.
- Do not change the alpha channel of a UI texture or mask as a side effect of generic image conversion.
- Do not add generated files, large cache directories, OS-specific binaries, or private configuration to source control unintentionally.
- Never delete unknown files merely because they are not referenced by current code.
- Do not run destructive `git reset --hard`, `git clean -fd`, broad force-checkouts, or equivalent commands to remove someone else's changes.
- When the work spans more than one module, explain the dependency and initialization order so the repository remains runnable.

After editing:

1. Inspect `git diff --check`, `git diff --stat`, and `git diff` (or targeted equivalent).
2. Check that edits did not create broken paths, unused assets, duplicated GPU resources, or configuration drift.
3. Run the checks applicable to the change; address introduced failures.
4. Summarize exactly which files changed, why, and which tests ran.

### Asset-specific checks

- **PNG/WebP:** decoder succeeds; width/height and channel count are as expected; alpha exists when needed; masks line up with base textures at the intended UV mapping; no surprise lossy conversion.
- **Grayscale masks:** sample data in the intended **linear** space, not through a color/sRGB conversion that changes mask thresholds.
- **Hit masks:** visible shape and interactive shape are independently defined when needed; verify transparent corners and any 9-slice UV remapping.
- **WGSL:** parse and validate with Naga (or the shader validator used by the project); verify all binding layouts against Rust/wgpu definitions. A validator passing does **not** prove the final visual effect works.
- **glTF/GLB:** validate structure, scene transforms, dependencies, and scale/orientation; check runtime loading and at least one representative render when possible.
- **RON/TOML/JSON:** parse or deserialize into real structures; validate ranges, required fields, versioning, referenced paths, and fallback behavior.
- **Fonts:** verify the actual glyphs needed by the game, including Ukrainian text when applicable.

## 15. Hot reload and developer iteration

Implement hot reload only when it improves iteration speed for an active feature. `notify` is the preferred file watcher for development builds, but it must not be a runtime dependency of shipped gameplay unless specifically needed.

If hot reload is implemented:

1. Watch only the configured asset/shader directories; debounce/coalesce rapid save events.
2. Load, decode, parse, and validate replacements **before** publishing them to the running game.
3. Perform GPU resource/pipeline creation at the correct render-thread/device ownership boundary.
4. Swap in validated resources atomically at a safe frame boundary.
5. On reload failure, retain the **last known-good** texture, material, shader, or configuration and report a clear error.
6. Rebuild only affected bindings, pipelines, and render resources; do not reset the entire game/UI state.
7. Preserve live interaction state (hover animation, input, navigation, world state) unless a deliberate reset is requested.
8. Ensure release builds work without editable source files, a watcher, or development-only paths.

Initially prioritize hot reload for **UI textures, glow/hit masks, material parameters, and WGSL**. Add model, scene, or audio hot reload only if game development actually benefits.

Live debug controls and disk hot reload must have a defined precedence (for example: explicit debug override until cleared; otherwise loaded asset value). Never silently persist an override.

## 16. Validation, testing, and definition of done

Run relevant verification commands from the repository root. Start with the checks that fit the actual change; for substantial changes prefer the full set:

```bash
cargo fmt --all -- --check
cargo check --all-targets
cargo test
cargo clippy --all-targets -- -D warnings
cargo build
```

If `cargo fmt -- --check` fails only because new code needs formatting, run `cargo fmt --all` and re-check; avoid reformatting unrelated files. Respect project feature flags, `--locked` where applicable, and the target platform's requirements.

Additional checks where appropriate:

```bash
git status --short
git diff --check
git diff --stat
```

**Automated tests to prioritize:**

- Tween interpolation, reversal, interruption from current state, and large/zero delta-time cases.
- Alpha/custom-mask hit testing, transformed coordinates, transparent pass-through, stacking order, clipping, and 9-slice mapping.
- Layout resize and DPI conversion, visible/invisible and disabled elements.
- Asset decode, missing/corrupt resources, cache consistency, serialized configuration and saves.
- Shader parameter packing, expected bind-group layouts, and shader validation.
- Fixed-step simulation/state transitions and UI input consumption.

**Manual/visual acceptance on GPU where possible:**

- 2D/3D depth and layering, transparent sprites, scene postprocessing, and text readability.
- Stone button's actual base artwork, crystal glow mask alignment, hover and mouse-leave transition.
- No hover/click in transparent button corners; correct click behavior on the visible shape.
- Hover exit mid-animation and rapid menu switches do not create visual jumps.
- A list of masked-material buttons fades smoothly from 0 to 1 through proper group composition, including overlapping children.
- Resize, DPI, minimize/focus loss, startup, shutdown, device/surface errors.
- Windows/Linux behavior, at least with targeted builds and smoke tests when available.
- Hot reload changes an asset without restarting, and **invalid** updates preserve last known-good resources.
- No unexpected per-frame pipeline creation, repetitive uploads, or uncontrolled GPU memory growth.

A command returning success proves only that command completed. **Do not claim visual correctness from compilation or unit tests alone.** If the agent cannot run a GPU window or does not possess the original art or a target OS, state exactly what remains unverified.

A task is complete only when the requested behavior is implemented, the relevant checks pass (or blockers are transparently reported), existing features are not knowingly broken, and the user can see how to run or test the result.

## 17. Agent workflow and communication

For each request:

1. **Inspect:** read this file, relevant repo files, and existing state; identify user-owned changes.
2. **Plan:** for nontrivial work, state a short, concrete plan with affected modules and acceptance criteria.
3. **Implement:** deliver the smallest complete, runnable vertical slice; integrate rather than duplicate subsystems.
4. **Review:** inspect diffs and assets; identify unintentional changes and compatibility risks.
5. **Verify:** run appropriate checks; use visual/GPU testing for visual changes where possible.
6. **Fix:** address errors introduced by the task and re-run relevant tests.
7. **Report in Ukrainian:** what changed, key design decisions, files touched, commands/tests and their real outcomes, remaining risks, and the next useful step.

**Never**:

- Turn the project into a general-purpose engine, a standalone editor, or a generic GUI framework.
- Introduce a web stack for the shipped UI without an explicit user decision.
- Replace user artwork or silently change masked materials to translucent.
- Add features unrelated to the present game goal or invent missing gameplay requirements.
- Make wide unrelated refactors or overwrite user changes.
- Claim completed testing, GPU verification, or performance measurements that were not performed.
- Promise asynchronous follow-up work that cannot actually be executed.

Questions should be reserved for truly blocking design decisions; for ordinary implementation details choose a small reversible default and document it.

## 18. Delivery roadmap — working game first

Advance through feature-driven milestones rather than implementing all subsystems up front:

1. **Boot and foundation:** working winit/wgpu app, event loop, logging, resize handling, asset loading and error paths.
2. **First interactive menu:** actual fantasy button base texture, glow mask shader, alpha-aware hover/click, correct state animation.
3. **Menu transitions:** multiple buttons, text, settings/navigation, masked-material group fading and input routing.
4. **Hybrid scene vertical slice:** one *playable* 2D/3D scene with a camera, correct world/UI layering, interaction, and minimal game rules.
5. **Core gameplay:** game states, audio/physics as required, settings, persistence if needed.
6. **Developer iteration:** targeted egui debug overlay, asset/ shader hot reload, robust error reporting when justified by workflow.
7. **Content and polish:** real game mechanics and assets, UI responsiveness, visual and CPU/GPU profiling, bug fixes.
8. **Release:** reproducible package, included assets, stable config/save paths, Windows/Linux smoke checks, no debug-only dependencies at runtime.

**Definition of useful progress:** a runnable game build with a real player-visible/gameplay improvement, not an elaborate directory hierarchy of unused engine systems.
