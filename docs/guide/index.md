# Introduction

sciart draws science and astronomy in your terminal: galaxies that collide, planets that
keep Kepler's time, a nerve that fires, sand that finds the nodes of a vibrating plate.
There are 85 scenes in 11 categories, each a small simulation drawn in braille dots and
half-block cells at 30 frames per second. While it runs, it keeps your machine awake.

Every scene is a real model, simplified to fit a terminal, and every one carries a short
science title in its bottom-left corner that names the phenomenon and the mechanism. The
scenes are deterministic: the same seed and the same terminal size give the same pictures
on the same machine.

This guide covers:

- [Installing](installing.md) the program.
- [Running](running.md) it: cycling through the scenes, picking one, choosing themes.
- The [keys](keys.md) you can press while it runs.
- The [configuration](configuration.md) file.
- [Colour and terminals](terminals.md): colour modes, colour-blind viewing, keeping the
  machine awake.
- [Scripting](scripting.md): ending by itself, one frame for a shell greeting, the art
  while another command runs, completions and manual pages.
- [Recording](recording.md) a scene to a GIF, PNG frames or an asciicast.
- Using sciart as a Rust [library](library.md).

You can also watch the scenes on this site without installing anything: the
[player](../play/index.html) runs them in your browser, and the
[gallery](../gallery/index.html) shows a picture of every scene.
