![](https://cdn.jsdelivr.net/gh/qleverty/pics/cue_.png)

A tiny always-on-top widget to keep your tasks in focus. One main task, and a list of parallel ones — that's it.

Built with Rust + egui.

---

## Usage

- **✓** — mark the main task as done (the first one from the list takes its place)
- **Click any sub-task** — promotes it to main, the old one goes back to the list
- **× button** — removes a sub-task
- **+ Add...** — adds a new sub-task
- **Drag** the dotted bar at the top to move the widget around

Tasks are saved automatically to `tasks.json` next to the executable.
