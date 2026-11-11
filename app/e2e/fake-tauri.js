// A pretend Rust side for the WebKit tests: runs in the page before the app, in place of Tauri.
//
// It answers the commands the interface sends with a small Vault held in memory: one Work,
// "Rain", whose text it keeps up to date from the editor's edits. Everything it's sent is
// logged in `window.__fake.log`, so a test can check what the interface asked for (a drag
// sent as a Move, a Comment with its range). Commands it doesn't know are answered with null
// and listed in `window.__fake.unknown`.

(() => {
  const now = Date.UTC(2026, 10, 9, 13, 0);
  const fake = {
    log: [],
    unknown: [],
    texts: {
      Rain: "= One\n\nThe rain had stopped. The street was wet.\n\nShe walked home.\n\n= Two\n\nMorning came slowly.\n\n= Three\n\nThe end.\n",
      // Another Work, to include in Rain.
      "Zampanò": "= Exploration A\n\nThe hallway was not there before.\n\n= The Navidson Record\n\nIt measured a quarter inch more inside than out.\n",
    },
    comments: [],
    /** What each Work's Inclusions include, in reading order (vault/inclusions.rs). */
    included: {},
    changes: 1,
    /** Mods each Work has (the Chapter list is on Rain). */
    mods: { Rain: ["chapters", "counts"] },
    /** Vault themes: file → { name, dark, css }. A test adds some before the page loads. */
    themes: {},
    settings: null,
    /** Android's shared storage, for the folder browser: each folder and the folders in it. */
    folders: {
      "/storage/emulated/0": ["Download", "Sync"],
      "/storage/emulated/0/Download": [],
      "/storage/emulated/0/Sync": ["Novels"],
      "/storage/emulated/0/Sync/Novels": [],
    },
    vaults: ["/storage/emulated/0/Sync/Novels"],
    syncthingFolders: ["/storage/emulated/0/Sync"],
    /** The Vault's Sync problems, as list_works reports them. */
    problems: [],
    /** What "Check for updates" finds (update.rs's Checked). */
    update: { kind: "newest", current: "0.1.2" },
    /** What "Uninstall…" finds before asking (lib.rs's UninstallCheck). */
    uninstallCheck: { problem: null, unsaved: [] },
  };
  window.__fake = fake;

  const settings = fake.settings = {
    theme: "light", fontSize: 17, themeHere: false, fontSizeHere: false,
    lineNumbers: false, lineNumbersHere: false, highlightLine: false, highlightLineHere: false,
    previewFollows: true, previewFollowsHere: false, commentHighlights: true, commentHighlightsHere: false,
    shortcuts: {}, shortcutsHere: false,
  };

  /** Applies one of the editor's edit ops to a Work's text (positions are UTF-16, as JS counts). */
  function apply(id, op) {
    let t = fake.texts[id];
    switch (op.op) {
      case "type":
      case "paste":
        t = t.slice(0, op.from) + op.text + t.slice(op.to);
        break;
      case "delete":
      case "cut":
        t = t.slice(0, op.from) + t.slice(op.to);
        break;
      case "move": {
        // `at` is where the text goes, counted in the text before the move.
        const piece = t.slice(op.from, op.to);
        const rest = t.slice(0, op.from) + t.slice(op.to);
        const at = op.at >= op.to ? op.at - (op.to - op.from) : op.at;
        t = rest.slice(0, at) + piece + rest.slice(at);
        break;
      }
      case "comment":
        fake.comments.push({
          id: `c${fake.comments.length + 1}`, ranges: [[op.from, op.to]], sender: op.sender, name: "Author", colour: "author",
          body: op.body, resolved: false, quote: t.slice(op.from, op.to), timeMs: now, replies: [], otherBody: null,
        });
        break;
      case "include": {
        // The Inclusion character the paste just put at `at`: the k-th one in the text.
        const k = [...t.slice(0, op.at)].filter((c) => c === "￼").length;
        const list = (fake.included[id] ??= []);
        list.splice(k, 0, { work: op.work, chapter: op.chapter });
        return false;
      }
      default:
        return false; // cursor, sync, marks, tints: nothing changes
    }
    fake.texts[id] = t;
    return true;
  }

  function workState(id, extra = {}) {
    return {
      len: fake.texts[id].length, patches: [], burst: null, saveError: null, textFileProblem: null, recoveryProblem: null,
      outside: 0, tookIn: null, syncProblem: null, moved: null, styleNotice: null, changes: fake.changes, notices: 0,
      marks: null, tints: null, ...extra,
    };
  }

  const handlers = {
    startup: () => ({ lastVault: "C:/Fake Vault" }),
    open_vault: ({ path }) => ({ status: "opened", root: path, deviceName: "Laptop", suggestedName: "Laptop" }),
    get_settings: () => settings,
    set_setting: ({ name, value }) => {
      settings[name] = name === "fontSize" ? Number(value) : value === "on" ? true : value === "off" ? false : value;
      return settings;
    },
    list_works: () => ({ entries: Object.keys(fake.texts).map((id) => ({ id, name: id, kind: "work" })), problems: fake.problems, folders: [] }),
    list_trash: () => [],
    get_library: () => ({ items: [], conflicts: [], builtIn: [] }),
    get_layout: () => ({ tabs: ["Rain"], active: "Rain", cursors: {} }),
    save_layout: () => null,
    open_work: ({ id }) => ({ id, name: id, text: fake.texts[id], problems: [], notice: null }),
    leave_work: () => null,
    close_work: () => null,
    get_mods: ({ id }) => ({ work: fake.mods[id] ?? [], newWorks: [], hidden: [] }),
    get_reviewers: () => [],
    get_preamble: () => ({ preamble: { looks: ["Paperback"], styles: [], elements: [], title: "Rain", author: "", language: "en", raw: "" }, rawOther: null }),
    get_milestones: () => [],
    list_themes: () => ({ themes: Object.entries(fake.themes).map(([file, t]) => ({ file, name: t.name, dark: t.dark })), problems: [] }),
    read_theme: ({ file }) => {
      if (!fake.themes[file]) throw new Error("No such theme");
      return fake.themes[file].css;
    },
    sync_status: () => ({ kind: "notRunning", line: "Syncthing isn't running", address: "http://127.0.0.1:8384" }),
    // Headings only, as Typst's highlighter would tag them: enough to see the colouring arrive.
    colour_typst: ({ text }) => {
      const found = [];
      let at = 0;
      for (const line of text.split("\n")) {
        if (line.startsWith("= ")) found.push({ from: at, to: at + line.length, class: "typ-heading" });
        at += line.length + 1;
      }
      return found;
    },
    complete_at: () => null,
    missing_inclusions: () => fake.missingInclusions ?? [],
    export_pdf: () => null, // as if the save dialog was cancelled
    compile_preview: () => ({ pages: [], stale: false, errors: [], warnings: [], deviceFonts: [], ms: 1 }),
    forget_preview: () => null,
    preview_locate: () => null,
    get_history: () => ({ sessions: [], milestones: [], notices: [], more: false }),
    edit: ({ id, ops }) => {
      let changed = false;
      for (const op of ops) changed = apply(id, op) || changed;
      if (changed) fake.changes += 1;
      const wantsMarks = ops.some((op) => op.op === "marks" || op.op === "comment");
      return workState(id, wantsMarks ? { marks: fake.comments } : {});
    },
    work_text: ({ id }) => ({ text: fake.texts[id], state: workState(id) }),
    // Inclusions (vault/inclusions.rs): a Work's Chapters, and what its Inclusions show.
    chapters_of: ({ work }) => {
      const found = [];
      let at = 0;
      for (const line of fake.texts[work].split("\n")) {
        if (line.startsWith("= ")) found.push({ title: line.slice(2), at });
        at += line.length + 1;
      }
      return found;
    },
    get_inclusions: ({ id }) => {
      const text = fake.texts[id];
      const list = fake.included[id] ?? [];
      const views = [];
      let k = 0;
      for (let at = 0; at < text.length; at++) {
        if (text[at] !== "￼") continue;
        const target = list[k++];
        if (!target) continue;
        const source = fake.texts[target.work];
        let shown = source;
        let label = target.work;
        if (target.chapter !== null) {
          const next = source.indexOf("\n= ", target.chapter + 1);
          shown = source.slice(target.chapter, next === -1 ? source.length : next + 1);
          label = `${target.work}: ${shown.split("\n")[0].slice(2)}`;
        }
        views.push({ at, work: target.work, label, text: shown.trimEnd(), missing: null, language: "en", whole: target.chapter === null });
      }
      return views;
    },
    list_folders: ({ path }) => {
      if (!fake.folders[path]) throw new Error(`Couldn't open ${path}`);
      const marks = (p) => ({ vault: fake.vaults.includes(p), syncthing: fake.syncthingFolders.includes(p) });
      return {
        path,
        parent: path.slice(0, path.lastIndexOf("/")) || "/",
        ...marks(path),
        folders: fake.folders[path].map((name) => ({ name, path: `${path}/${name}`, ...marks(`${path}/${name}`) })),
      };
    },
    app_hidden: () => [],
    app_info: () => ({ version: "0.1.2", canUninstall: true }),
    uninstall_check: () => fake.uninstallCheck,
    uninstall: () => null,
    check_update: () => fake.update,
    install_update: () => ({ kind: "restarting" }),
    open_release_page: () => null,
    "plugin:event|listen": () => 0,
    "plugin:event|unlisten": () => null,
  };

  let nextCallback = 1;
  const callbacks = new Map();
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
    transformCallback(callback) {
      const id = nextCallback++;
      callbacks.set(id, callback);
      return id;
    },
    unregisterCallback: (id) => callbacks.delete(id),
    convertFileSrc: (path) => path,
    async invoke(cmd, args = {}) {
      fake.log.push({ cmd, args: JSON.parse(JSON.stringify(args ?? {})) });
      const handler = handlers[cmd];
      if (!handler) {
        fake.unknown.push(cmd);
        return null;
      }
      return handler(args);
    },
  };
})();
