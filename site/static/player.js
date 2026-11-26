// The live player (WEB-R3, WEB-R5): sciart's scenes, compiled to WebAssembly, drawn on a
// canvas. The module is this site's own file and needs nothing else (WEB-R4, WEB-R6): the
// only address this script loads is the module's path from the page's own data. The page's
// time comes from the animation frame; the module turns it into fixed 1/60 s steps. Where
// WebAssembly is missing or the module fails at any point, the recorded clip comes back.
"use strict";

(function () {
  const data = JSON.parse(document.getElementById("player-data").textContent);
  const root = document.getElementById("player");
  const stage = document.getElementById("stage");
  const fallback = document.getElementById("fallback");
  const fallbackImg = fallback.querySelector("img");
  const fallbackNote = document.getElementById("fallback-note");
  const screen = document.getElementById("screen");
  const controls = document.getElementById("controls");
  const status = document.getElementById("status");
  const sceneSelect = document.getElementById("scene");
  const themeSelect = document.getElementById("theme");
  const seedInput = document.getElementById("seed");
  const playButton = document.getElementById("play");
  const restartButton = document.getElementById("restart");
  const reseedButton = document.getElementById("reseed");

  const params = new URLSearchParams(window.location.search);
  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  let failed = false;

  // Show the recorded clip of `scene` when there is one, and stop the live player.
  function showFallback(scene, reason) {
    failed = true;
    if (data.clips[scene]) {
      fallbackImg.src = data.clips[scene].src;
      fallbackImg.alt = data.clips[scene].alt;
    }
    fallbackNote.textContent = reason;
    fallback.hidden = false;
    screen.hidden = true;
    controls.hidden = true;
    status.textContent = "";
  }

  if (typeof WebAssembly !== "object" || typeof WebAssembly.instantiate !== "function") {
    showFallback(params.get("scene") || data.default_scene,
      "This browser cannot run WebAssembly, so here is a recorded clip instead.");
    return;
  }

  // ---- The module, wrapped in a small Player class --------------------------------------

  function load() {
    return fetch(data.module)
      .then((response) => {
        if (!response.ok) throw new Error("the player module did not load (" + response.status + ")");
        return response.arrayBuffer();
      })
      .then((bytes) => WebAssembly.instantiate(bytes, {}))
      .then((result) => result.instance.exports);
  }

  function bind(wasm) {
    const encoder = new TextEncoder();
    const decoder = new TextDecoder();
    const output = () => decoder.decode(new Uint8Array(wasm.memory.buffer, wasm.output_ptr(), wasm.output_len()));
    const input = (text) => {
      const bytes = encoder.encode(text);
      const ptr = wasm.input(bytes.length);
      new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
    };

    class Player {
      // new(scene, columns, rows, seed): throws for an unknown scene.
      constructor(scene, columns, rows, seed) {
        input(scene);
        const s = Math.max(0, Math.floor(Number(seed) || 0));
        this.handle = wasm.player_new(columns, rows, Math.floor(s / 4294967296) >>> 0, s >>> 0);
        if (this.handle === 0) throw new Error("no scene named " + scene);
        wasm.player_scene(this.handle);
        this.scene = output();
      }
      advance(seconds) { return wasm.player_advance(this.handle, seconds) === 1; }
      resize(columns, rows) { wasm.player_resize(this.handle, columns, rows); }
      // frame(theme): RGBA pixels as an ImageData, theme by name.
      frame(theme) {
        const index = Math.max(0, Player.themes().indexOf(theme));
        const ptr = wasm.player_frame(this.handle, index);
        const width = wasm.player_width(this.handle);
        const height = wasm.player_height(this.handle);
        const pixels = new Uint8ClampedArray(wasm.memory.buffer, ptr, width * height * 4);
        return new ImageData(pixels, width, height);
      }
      free() { wasm.player_free(this.handle); this.handle = 0; }
      static scenes() { wasm.scenes_json(); return JSON.parse(output()).scenes; }
      static themes() {
        if (!Player.themeNames) { wasm.theme_names(); Player.themeNames = output().split("\n"); }
        return Player.themeNames;
      }
    }
    return Player;
  }

  // ---- The page --------------------------------------------------------------------------

  load().then((wasm) => {
    const Player = bind(wasm);
    const scenes = Player.scenes();
    const byName = new Map(scenes.map((s) => [s.name, s]));

    // Scene choice, grouped by category in the gallery's domain order, which the page's data
    // gives (WEB-R2); a category it does not name comes last, by label.
    const order = Array.isArray(data.category_order) ? data.category_order : [];
    const rank = (category) => {
      const i = order.indexOf(category);
      return i < 0 ? order.length : i;
    };
    const groups = new Map();
    for (const s of scenes) {
      if (!groups.has(s.category)) groups.set(s.category, []);
      groups.get(s.category).push(s);
    }
    const sorted = [...groups].sort((a, b) =>
      rank(a[0]) - rank(b[0]) || a[1][0].category_label.localeCompare(b[1][0].category_label));
    for (const [, members] of sorted) {
      const group = document.createElement("optgroup");
      group.label = members[0].category_label;
      for (const s of members) group.append(new Option(s.name.replace(/_/g, " "), s.name));
      sceneSelect.append(group);
    }
    for (const t of Player.themes()) themeSelect.append(new Option(t, t));
    // `n` and `p` walk the scenes in the order the list shows them.
    const names = [...sceneSelect.options].map((o) => o.value);

    const state = {
      scene: names.includes(params.get("scene")) ? params.get("scene") : data.default_scene,
      theme: Player.themes().includes(params.get("theme")) ? params.get("theme") : "astro",
      seed: /^\d{1,15}$/.test(params.get("seed") || "") ? Number(params.get("seed")) : 42,
      playing: !reduceMotion.matches,
      time: 0,
    };
    let player = null;
    let last = null;
    const ctx = screen.getContext("2d");

    function fail(error) {
      showFallback(state.scene, "The live player stopped (" + error.message + "); here is a recorded clip instead.");
    }

    // Every way into the module goes through here: an error or a trap brings the clip back.
    function guarded(action) {
      return function (...args) {
        if (failed) return;
        try { action.apply(this, args); } catch (error) { fail(error); }
      };
    }

    function cells() {
      const width = stage.clientWidth || 800;
      const columns = Math.max(40, Math.min(100, Math.floor(width / 8)));
      const rows = Math.max(12, Math.min(30, Math.round(columns * 0.3)));
      return [columns, rows];
    }

    function share() {
      const q = new URLSearchParams({ scene: state.scene, theme: state.theme, seed: String(state.seed) });
      window.history.replaceState(null, "", "?" + q.toString());
    }

    function draw() {
      const image = player.frame(state.theme);
      if (screen.width !== image.width || screen.height !== image.height) {
        screen.width = image.width;
        screen.height = image.height;
      }
      ctx.putImageData(image, 0, 0);
      const info = byName.get(player.scene);
      status.textContent = "";
      const title = document.createElement("span");
      title.className = "title";
      title.textContent = info.title;
      status.append(title, " · t = " + state.time.toFixed(1) + " s · seed " + state.seed +
        (state.playing ? "" : " · paused"));
    }

    function start() {
      if (player) player.free();
      player = null;
      const [columns, rows] = cells();
      player = new Player(state.scene, columns, rows, state.seed);
      state.time = 0;
      if (!state.playing) {
        // A still that shows the scene, not its empty first instant.
        player.advance(5);
        state.time = 5;
      }
      sceneSelect.value = state.scene;
      themeSelect.value = state.theme;
      seedInput.value = String(state.seed);
      playButton.textContent = state.playing ? "Pause" : "Play";
      share();
      draw();
    }

    const tick = guarded(function tick(now) {
      if (!state.playing || !player) { last = null; return; }
      if (last !== null) {
        // A hidden tab or a slow frame must not jump the scene forward.
        const dt = Math.min(Math.max((now - last) / 1000, 0), 0.25);
        player.advance(dt);
        state.time += dt;
        draw();
      }
      last = now;
      window.requestAnimationFrame(tick);
    });

    const setPlaying = guarded(function setPlaying(playing) {
      state.playing = playing;
      playButton.textContent = playing ? "Pause" : "Play";
      if (playing) { last = null; window.requestAnimationFrame(tick); } else { draw(); }
    });

    const step = guarded(function step(delta) {
      const i = names.indexOf(state.scene);
      state.scene = names[(i + delta + names.length) % names.length];
      start();
    });

    const nextTheme = guarded(function nextTheme() {
      const themes = Player.themes();
      state.theme = themes[(themes.indexOf(state.theme) + 1) % themes.length];
      themeSelect.value = state.theme;
      share();
      draw();
    });

    const restart = guarded(start);

    sceneSelect.addEventListener("change", guarded(() => { state.scene = sceneSelect.value; start(); }));
    themeSelect.addEventListener("change", guarded(() => { state.theme = themeSelect.value; share(); draw(); }));
    seedInput.addEventListener("change", guarded(() => {
      const seed = Math.floor(Number(seedInput.value));
      if (Number.isFinite(seed) && seed >= 0 && seed <= Number.MAX_SAFE_INTEGER) { state.seed = seed; start(); }
      else { seedInput.value = String(state.seed); }
    }));
    playButton.addEventListener("click", () => setPlaying(!state.playing));
    restartButton.addEventListener("click", restart);
    reseedButton.addEventListener("click", guarded(() => { state.seed = Math.floor(Math.random() * 1e9); start(); }));
    controls.addEventListener("submit", (event) => event.preventDefault());

    // Keys, as in the terminal: space, n/p (or the arrows), t, r. Only while focus is inside
    // the player, and never instead of a focused control's own action: a field keeps every
    // key, and a button, link or summary keeps space, Enter and the arrows.
    root.addEventListener("keydown", (event) => {
      if (event.altKey || event.ctrlKey || event.metaKey || failed) return;
      const tag = event.target.tagName;
      if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;
      const onControl = tag === "BUTTON" || tag === "A" || tag === "SUMMARY";
      const key = event.key;
      if (key === "n" || key === "p" || key === "t" || key === "r") {
        if (key === "n") step(1);
        else if (key === "p") step(-1);
        else if (key === "t") nextTheme();
        else restart();
      } else if (!onControl && key === " ") {
        event.preventDefault();
        setPlaying(!state.playing);
      } else if (!onControl && (key === "ArrowRight" || key === "ArrowLeft")) {
        event.preventDefault();
        step(key === "ArrowRight" ? 1 : -1);
      }
    });

    let resizeTimer = 0;
    window.addEventListener("resize", () => {
      window.clearTimeout(resizeTimer);
      resizeTimer = window.setTimeout(guarded(() => {
        if (!player) return;
        const [columns, rows] = cells();
        player.resize(columns, rows);
        draw();
      }), 150);
    });

    fallback.hidden = true;
    screen.hidden = false;
    controls.hidden = false;
    restart();
    if (failed) return;
    if (state.playing) window.requestAnimationFrame(tick);
    else status.append(" · your system asks for reduced motion; press Play, or space on the picture, to start");
  }).catch((error) => {
    showFallback(params.get("scene") || data.default_scene,
      "The live player could not start (" + error.message + "); here is a recorded clip instead.");
  });
})();
