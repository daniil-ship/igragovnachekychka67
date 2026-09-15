# ECHO OF FORGOTTEN WALLS

**A first-person psychological horror in three acts. The bunker remembers what you did.**

> *Arthur, 34, archivist. Archive-404. Final inventory.*
> *You left your little brother Misha alone in the dark — and the dark never forgot.*

---

## ABOUT

**Echo of Forgotten Walls** is a story-driven horror game built with **Bevy 0.16 (Rust, ECS)**. Every act generates a brand-new procedural labyrinth: pull the generator lever, gather 5 echo fragments in total darkness, then outrun a 3-minute reactor meltdown to reach the lift.

But you are not alone down here. The bunker materializes your guilt as a living thing — **The Fade**, a shadow that patrols, stares... and hunts. Your flashlight is your only weapon: hold the beam on it, or madness will boil over into a full-screen screamer.

Scattered across all three acts are **5 keepsakes of your brother** — a rusty toy car, a child's drawing, a wool scarf, a torn photo, a small mitten. Find them all before entering the lift, and earn **REDEMPTION**. Enter without them... and face **ABSORPTION**.

---

## FEATURES

- 🎭 **Three hand-paced acts** — The Descent, The Rejection, The Reactor — each with its own maze, lighting and textures (Act 2 bleeds, Act 3 is scorched)
- 👁️ **Living madness system** — sprinting on empty stamina, darkness, and the monster's touch feed your insanity; tunnel vision, shaking hands and whispering walls follow
- 🔦 **Physical flashlight** — inertia, sway, dying-battery flicker; its cone is the only thing the Fade fears
- 👹 **A stalking monster with real AI** — patrols, stares, chases, retreats from direct light (line-of-sight checked)
- 🎞️ **Custom WGSL post-processing** — film grain and a vignette that thickens with your madness
- 🔊 **Positional 3D audio anomalies** — footsteps and whispers *behind* you when insanity runs high (headphones recommended!)
- 📼 **Story tapes, batteries, subtitles** — fully voiced-through-text lore of Prof. Radchenko's 1984 expedition
- 💾 **Save/load system** (F5/F9) — the exact same maze, restored down to the last wall
- 🖥️ **Fullscreen support** (F11), film-grain/vignette toggles in the menu
- ⚡ **Zero HUD clutter** — no counters, no markers. Only a dim stamina bar and your own fear

---

## CONTROLS

| Key | Action |
|---|---|
| WASD / Arrows | Move |
| Mouse | Look |
| Shift | Sprint (drains stamina) |
| F | Flashlight on/off (drains battery) |
| E | Use: tapes / generator lever |
| F5 / F9 | Save / load game |
| F11 | Fullscreen |
| R | Restart run from Act 1 |
| Esc | Menu |

---

## TIPS FOR SURVIVAL

- 🩸 **Darkness feeds madness** — never let the torch die. Green batteries restore +40 charge.
- 🏃 **Sprinting on empty stamina** keeps you fast but drives you insane (+25/s). Rest in the light.
- 🔦 **Hold your light on the shadow** — it cannot stand direct light, but it learns to wait.
- 🧸 **Find all 5 keepsakes** for the good ending. Check every dead end.

---

## TECHNICAL NOTES

- Engine: **Bevy 0.16**, language: **Rust**
- All textures are **procedural** (generated in code) — the game runs with an empty `assets/` folder
- All sounds are optional: drop your own MP3s into `assets/audio/` and the game picks them up live, no restart needed
- Custom textures per act: `assets/textures/act2_*.png`, `assets/textures/act3_*.png`

---

*The lift is waiting. Misha is waiting. Don't run this time.*
