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
    /** Comments made on included text, by the Work they're kept in (positions in the shown text). */
    includedComments: {},
    /** Annotations (ADR 0020), by the Work that made them: on its Inclusion at `at`, `from..to` in its text. */
    annotations: {},
    changes: 1,
    /** Mods each Work has (the Chapter list is on Rain). */
    mods: { Rain: ["chapters", "counts"] },
    /** Mods' own settings (vault/marks.rs): the Vault's, each Work's, new Works'. */
    modSettings: { vault: {}, work: {}, newWorks: {} },
    /** Dismissed Marks, by Work. */
    dismissed: {},
    /** The Vault Mods the Vault has (none unless a test adds one). */
    vaultMods: [],
    /** The words the fake Dictionary doesn't know. */
    misspelt: ["Teh", "teh", "Kvothe", "recieve"],
    /** The Vault's folders, besides those its Works are in (none unless a test adds some). */
    vaultFolders: [],
    /** Each Work's Images (ADR 0021), by name in lowercase. */
    images: {},
    /** Each Work's Past pictures, by Image (name in lowercase), newest first. */
    past: {},
    /** How many pictures Insert Image's picker has handed out: each is a new one. */
    picked: 0,
    /** The colours of Works and folders (vault/colours.rs). */
    colours: { works: {}, folders: {} },
    /** Vault themes: file → { name, dark, css }. A test adds some before the page loads. */
    themes: {},
    settings: null,
    /** The Vault the app opens at the start (null: none, as on a first start). */
    lastVault: "C:/Fake Vault",
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
    /** The pages a compile gives (their sizes in points). None unless a test adds some. */
    pages: [],
  };
  window.__fake = fake;

  const settings = fake.settings = {
    theme: "light", fontSize: 17, themeHere: false, fontSizeHere: false,
    lineNumbers: false, lineNumbersHere: false, highlightLine: false, highlightLineHere: false,
    previewFollows: true, previewFollowsHere: false, commentHighlights: true, commentHighlightsHere: false,
    shortcuts: {}, shortcutsHere: false, hiddenFoldersHere: false,
    shared: { theme: "light", fontSize: 17, lineNumbers: false, highlightLine: false, previewFollows: true, commentHighlights: true },
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
      case "replace":
        // Find and replace: word edits in the text as it was, applied last first.
        for (const e of [...op.edits].sort((x, y) => y.from - x.from)) t = t.slice(0, e.from) + e.text + t.slice(e.to);
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
          id: `c${fake.comments.length + 1}`, ranges: [[op.from, op.to]], onWork: false, sender: op.sender, name: "Author", colour: "author",
          body: op.body, resolved: false, quote: t.slice(op.from, op.to), timeMs: now, replies: [], otherBody: null,
        });
        break;
      case "workComment":
        // Rust lists Work comments first, oldest first.
        fake.comments.splice(fake.comments.filter((c) => c.onWork).length, 0, {
          id: `c${fake.comments.length + 1}`, ranges: [], onWork: true, sender: op.sender, name: "Author", colour: "author",
          body: op.body, resolved: false, quote: "", timeMs: now, replies: [], otherBody: null,
        });
        break;
      case "setImage": {
        const key = op.name.toLowerCase();
        const had = fake.images[id]?.[key];
        const past = ((fake.past[id] ??= {})[key] ??= []);
        // The picture it had becomes a Past picture; one used again stops being one.
        fake.past[id][key] = past.filter((p) => p.hash !== op.hash);
        if (had && had.hash !== op.hash) fake.past[id][key].unshift({ hash: had.hash, givenMs: now, size: had.size });
        (fake.images[id] ??= {})[key] = { name: op.name, kind: "PNG", size: 77, hash: op.hash };
        return false;
      }
      case "deleteImage":
        delete fake.images[id]?.[op.name.toLowerCase()];
        return false;
      case "include": {
        // The Inclusion character the paste just put at `at`: the k-th one in the text.
        const k = [...t.slice(0, op.at)].filter((c) => c === "￼").length;
        const list = (fake.included[id] ??= []);
        list.splice(k, 0, { work: op.work, chapter: op.chapter });
        return false;
      }
      default:
        return false; // cursor, sync, highlights, tints, dismissed: nothing changes
    }
    fake.texts[id] = t;
    return true;
  }

  function modSettings(id) {
    const dismissed = {};
    for (const d of (id && fake.dismissed[id]) || []) {
      const mod = d.key.split("/")[0];
      dismissed[mod] = (dismissed[mod] ?? 0) + 1;
    }
    return { vault: fake.modSettings.vault, work: (id && fake.modSettings.work[id]) || {}, newWorks: fake.modSettings.newWorks, dismissed };
  }

  function workState(id, extra = {}) {
    return {
      len: fake.texts[id].length, patches: [], burst: null, saveError: null, textFileProblem: null, recoveryProblem: null,
      outside: 0, tookIn: null, syncProblem: null, moved: null, styleNotice: null, changes: fake.changes, notices: 0,
      highlights: null, tints: null, dismissed: null, ...extra,
    };
  }

  const handlers = {
    startup: () => ({ lastVault: fake.lastVault }),
    vault_folder: () => ({ vault: true, files: 0 }),
    open_vault: ({ path }) => ({ status: "opened", root: path, deviceName: "Laptop", suggestedName: "Laptop" }),
    get_settings: () => settings,
    set_setting: ({ name, value, here }) => {
      const v = name === "fontSize" ? Number(value) : value === "on" ? true : value === "off" ? false : value;
      settings[name] = v;
      if (!here) {
        settings.shared[name] = v;
        settings[`${name}Here`] = false;
      }
      return settings;
    },
    // Ticking keeps the value in use; unticking goes back to the shared one (vault/settings.rs).
    set_setting_here: ({ name, here }) => {
      settings[`${name}Here`] = here;
      if (!here) settings[name] = settings.shared[name];
      return settings;
    },
    list_works: () => {
      const ids = Object.keys(fake.texts);
      // Every folder a Work is in, and the folders those are in.
      const inFolders = ids.flatMap((id) => id.split("/").slice(0, -1).map((_, i, parts) => parts.slice(0, i + 1).join("/")));
      return {
        entries: ids.map((id) => ({ id, name: id.split("/").pop(), kind: "work" })),
        problems: fake.problems, folders: [...new Set([...fake.vaultFolders, ...inFolders])].sort(), hidden: [], colours: fake.colours,
      };
    },
    rename_work: ({ id, name }) => {
      fake.texts[name] = fake.texts[id];
      delete fake.texts[id];
      return name;
    },
    rename_folder: ({ path, name }) => {
      const moved = (p) => (p === path || p.startsWith(`${path}/`) ? name + p.slice(path.length) : p);
      for (const id of Object.keys(fake.texts)) {
        const to = moved(id);
        if (to !== id) {
          fake.texts[to] = fake.texts[id];
          delete fake.texts[id];
        }
      }
      fake.vaultFolders = fake.vaultFolders.map(moved);
      return [];
    },
    list_trash: () => [],
    get_library: () => ({
      items: [{ kind: "style", name: "Courier voice", title: "Courier voice", path: "styles/Courier voice.typ", description: "", problems: [], writtenAs: null, shownAs: null, docx: null, styles: [] }],
      conflicts: [], builtIn: [],
    }),
    get_layout: () => ({ tabs: ["Rain"], active: "Rain", cursors: {}, reading: fake.reading ?? null }),
    save_layout: () => null,
    open_work: ({ id }) => ({ id, name: id, text: fake.texts[id], problems: [], notice: null }),
    /** Find in Vault: a Work's text, stamped with how many edits the fake has had. */
    text_for_finding: ({ id, known }) => {
      if (!(id in fake.texts)) return { kind: "unreadable", reason: "isn't in the Vault any more" };
      const stamp = String(fake.changes);
      return known === stamp ? { kind: "unchanged" } : { kind: "text", stamp, text: fake.texts[id] };
    },
    /** The Reading panel: a Work's text, read without opening it (vault/reading.rs). */
    read_work: ({ work, known }) => {
      const name = work.split("/").pop();
      if (!(work in fake.texts)) return { kind: "unreadable", id: work, name, identity: null, reason: `${name} isn't in the Vault any more.` };
      const highlights = fake.readingMarks?.[work] ?? [];
      const stamp = `${fake.changes}:${JSON.stringify(highlights)}`;
      if (known === stamp) return { kind: "unchanged" };
      return { kind: "text", id: work, name, identity: `id-${name}`, stamp, text: fake.texts[work], inclusions: [], highlights, elements: [] };
    },
    leave_work: () => null,
    close_work: () => null,
    get_mods: ({ id }) => ({ work: fake.mods[id] ?? [], newWorks: [], vault: fake.vaultMods, hidden: [] }),
    set_mod: ({ id, name, on }) => {
      const list = fake.mods[id] ?? [];
      fake.mods[id] = on ? [...new Set([...list, name])] : list.filter((m) => m !== name);
      return { work: fake.mods[id], newWorks: [], vault: fake.vaultMods, hidden: [] };
    },
    // Mods' own settings (vault/marks.rs).
    get_mod_settings: ({ id }) => modSettings(id),
    set_mod_setting: ({ scope, id, key, value }) => {
      const map = scope === "work" ? (fake.modSettings.work[id] ??= {}) : fake.modSettings[scope];
      if (value) map[key] = value;
      else delete map[key];
      return modSettings(id);
    },
    show_dismissed: ({ id, name }) => {
      fake.dismissed[id] = (fake.dismissed[id] ?? []).filter((d) => !d.key.startsWith(`${name}/`));
      return modSettings(id);
    },
    // The Spellchecker (spelling.rs): a British English Dictionary that knows every word but a few.
    spell_check: ({ words }) => words.filter((w) => fake.misspelt.includes(w)),
    spell_suggest: ({ word }) => ({
      teh: ["the", "tea"],
      recieve: ["receive", "relieve", "deceive", "reprieve", "recede", "receiver", "recite"],
    })[word.toLowerCase()] ?? [],
    dictionary_for: () => ({ code: "en-GB", name: "English (United Kingdom)", builtIn: true, insteadOf: null, offer: null }),
    list_dictionaries: () => ({
      inVault: [],
      builtIn: [{ code: "en-GB", name: "English (United Kingdom)", license: "(MIT AND BSD)", update: false }],
      offered: [{ code: "da", name: "Danish", license: "GPL-2.0 OR LGPL-2.1 OR MPL-1.1", size: 3800884 }],
    }),
    download_dictionary: () => null,
    remove_dictionary: () => null,
    set_vault_mod: ({ name, on }) => {
      fake.vaultMods = on ? [...new Set([...fake.vaultMods, name])] : fake.vaultMods.filter((m) => m !== name);
      return { work: [], newWorks: [], vault: fake.vaultMods, hidden: [] };
    },
    set_work_colour: ({ id, colour }) => {
      if (colour) fake.colours.works[id] = colour;
      else delete fake.colours.works[id];
      return null;
    },
    get_reviewers: () => [],
    // Newer versions of the app's default Styles to offer (library.rs): none here.
    default_offers: () => [],
    get_preamble: () => ({ preamble: { looks: ["Paperback"], styles: [], elements: [], title: "Rain", author: "", language: "en", raw: "" }, rawOther: null }),
    get_milestones: () => fake.milestones ?? [],
    make_milestone: ({ name }) => {
      (fake.milestones ??= []).push({ id: `00000001:${90 + fake.milestones.length}`, name, timeMs: now });
      return null;
    },
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
    comment_on_inclusion: ({ id, at, from, to, sender, body }) => {
      const target = fake.included[id][[...fake.texts[id].slice(0, at)].filter((c) => c === "￼").length];
      const list = (fake.includedComments[target.work] ??= []);
      const quote = handlers.get_inclusions({ id }).find((v) => v.at === at).text.slice(from, to);
      list.push({ id: `ic${list.length + 1}`, from, to, sender, body, quote, replies: [] });
      return null;
    },
    annotate: ({ id, at, from, to, body }) => {
      const list = (fake.annotations[id] ??= []);
      const quote = handlers.get_inclusions({ id }).find((v) => v.at === at).text.slice(from, to);
      const made = `an${list.length + 1}`;
      list.push({ id: made, at, from, to, body: body.trim(), quote });
      return made;
    },
    set_annotation: ({ id, annotation, body }) => {
      (fake.annotations[id] ?? []).find((a) => a.id === annotation).body = body.trim();
      return null;
    },
    delete_annotation: ({ id, annotation }) => {
      fake.annotations[id] = (fake.annotations[id] ?? []).filter((a) => a.id !== annotation);
      return null;
    },
    detached_annotations: () => [],
    get_images: ({ id }) => {
      const text = fake.texts[id] ?? "";
      const own = Object.values(fake.images[id] ?? {}).map((image) => {
        const uses = [];
        for (const m of text.matchAll(/image\(\s*"([^"\n]*)"/g)) {
          if (m[1].toLowerCase() === image.name.toLowerCase()) uses.push(m.index + m[0].indexOf('"') + 1);
        }
        const pastPictures = fake.past[id]?.[image.name.toLowerCase()] ?? [];
        return { ...image, uses, past: pastPictures.length, pastSize: pastPictures.reduce((n, p) => n + p.size, 0), pastPictures, standIn: false };
      });
      return { own, included: [], pastFolder: "" };
    },
    // Insert Image's file picker: always the same small picture, "map.png".
    pick_image: ({ id }) => {
      const had = fake.images[id]?.["map.png"];
      const hash = String(++fake.picked).padStart(64, "0");
      return { hash, name: "map.png", kind: "PNG", size: 77, clash: had ? had.name : null, otherName: "map 2.png", same: had?.hash === hash };
    },
    past_picture: () => handlers.image_picture(),
    delete_past_pictures: ({ id, hashes }) => {
      let deleted = 0;
      for (const [key, list] of Object.entries(fake.past[id] ?? {})) {
        fake.past[id][key] = list.filter((p) => !hashes.includes(p.hash) || (deleted++, false));
      }
      return deleted;
    },
    // A red 300×200 PNG: tall enough to click its lower part.
    image_picture: () => Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAASwAAADICAIAAADdvUsCAAACRUlEQVR4nO3TMQEAIAzAMEDI/EtBFjJ6kCjo031nFtA5dQD8zoQQMyHETAgxE0LMhBAzIcRMCDETQsyEEDMhxEwIMRNCzIQQMyHETAgxE0LMhBAzIcRMCDETQsyEEDMhxEwIMRNCzIQQMyHETAgxE0LMhBAzIcRMCDETQsyEEDMhxEwIMRNCzIQQMyHETAgxE0LMhBAzIcRMCDETQsyEEDMhxEwIMRNCzIQQMyHETAgxE0LMhBAzIcRMCDETQsyEEDMhxEwIMRNCzIQQMyHETAgxE0LMhBAzIcRMCDETQsyEEDMhxEwIMRNCzIQQMyHETAgxE0LMhBAzIcRMCDETQsyEEDMhxEwIMRNCzIQQMyHETAgxE0LMhBAzIcRMCDETQsyEEDMhxEwIMRNCzIQQMyHETAgxE0LMhBAzIcRMCDETQsyEEDMhxEwIMRNCzIQQMyHETAgxE0LMhBAzIcRMCDETQsyEEDMhxEwIMRNCzIQQMyHETAgxE0LMhBAzIcRMCDETQsyEEDMhxEwIMRNCzIQQMyHETAgxE0LMhBAzIcRMCDETQsyEEDMhxEwIMRNCzIQQMyHETAgxE0LMhBAzIcRMCDETQsyEEDMhxEwIMRNCzIQQMyHETAgxE0LMhBAzIcRMCDETQsyEEDMhxEwIMRNCzIQQMyHETAgxE0LMhBAzIcRMCDETQsyEEDMhxEwIMRNCzIQQMyHETAgxE0LMhBAzIcRMCDETQsyEEDMhxEwIMRNCzIQQMyHETAgxE0LMhBAzIcQe3acClC+A44kAAAAASUVORK5CYII="), (c) => c.charCodeAt(0)).buffer,
    comment_in: ({ work, ops }) => {
      for (const op of ops) {
        const c = (fake.includedComments[work] ?? []).find((x) => x.id === op.comment);
        if (op.op === "reply" && c) c.replies.push({ id: `r${c.replies.length + 1}`, sender: op.sender, name: "Author", colour: "author", body: op.body, timeMs: now, otherBody: null });
      }
      return null;
    },
    export_pdf: () => null, // as if the save dialog was cancelled
    set_inclusion_styles: ({ id, at, styles }) => {
      const k = [...fake.texts[id].slice(0, at)].filter((c) => c === "￼").length;
      fake.included[id][k].styles = styles;
      return null;
    },
    settle_split: ({ id, at }) => {
      const k = [...fake.texts[id].slice(0, at)].filter((c) => c === "￼").length;
      fake.included[id][k].settled = true;
      return null;
    },
    compile_preview: () => ({ pages: fake.pages, stale: false, errors: [], warnings: [], deviceFonts: [], ms: 1 }),
    /** A plain page with its number on it. */
    preview_page: ({ page }) => {
      const { width, height } = fake.pages[page];
      const hash = `page-${page}`;
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}"><text x="40" y="80" font-size="24">Page ${page + 1}</text></svg>`;
      return { hash, svg };
    },
    forget_preview: () => null,
    preview_locate: () => null,
    get_history: () => ({ sessions: [], milestones: [], notices: [], more: false }),
    edit: ({ id, ops }) => {
      let changed = false;
      for (const op of ops) changed = apply(id, op) || changed;
      if (changed) fake.changes += 1;
      const wantsHighlights = ops.some((op) => op.op === "highlights" || op.op === "comment" || op.op === "workComment");
      // Dismissed Marks (ADR 0018): kept as ranges in the text (the real one keeps character IDs).
      for (const op of ops) if (op.op === "dismiss") (fake.dismissed[id] ??= []).push({ key: op.key, ranges: [[op.from, op.to]] });
      const wantsDismissed = ops.some((op) => op.op === "dismissed");
      return workState(id, {
        ...(wantsHighlights ? { highlights: fake.comments } : {}),
        ...(wantsDismissed ? { dismissed: fake.dismissed[id] ?? [] } : {}),
      });
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
    // A Work can't include itself (the fake knows no deeper loops).
    check_inclusion: ({ id, work }) => {
      if (id === work) throw "A Work can't include itself.";
      return null;
    },
    create_collection: ({ id, works }) => {
      fake.texts[id] = works.map(() => "￼").join("\n\n");
      fake.included[id] = works.map((work) => ({ work, chapter: null }));
      return { id, name: id.split("/").pop(), text: fake.texts[id], problems: [], notice: null };
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
        let chapter = null;
        if (target.chapter !== null) {
          const next = source.indexOf("\n= ", target.chapter + 1);
          shown = source.slice(target.chapter, next === -1 ? source.length : next + 1);
          chapter = shown.split("\n")[0].slice(2);
          label = `${target.work}: ${chapter}`;
        }
        const comments = (fake.includedComments[target.work] ?? []).map((c) => ({
          id: c.id, ranges: [[c.from, c.to]], sender: c.sender, name: "Author", colour: "author", body: c.body, resolved: false,
          quote: c.quote, timeMs: now, replies: c.replies, otherBody: null,
        }));
        const found = fake.splits?.[target.work];
        const split = found && found.after === target.chapter && !target.settled ? { title: found.title, at: found.at } : null;
        const annotations = (fake.annotations[id] ?? []).filter((a) => a.at === at).map((a) => ({
          id: a.id, owner: id, ownerLabel: id, mine: true, at: a.to, ranges: [[a.from, a.to]], body: a.body, quote: a.quote, notes: [],
        }));
        views.push({ at, work: target.work, label, chapter, text: shown.trimEnd(), missing: null, language: "en", whole: target.chapter === null, starts: target.chapter ?? 0, comments, split, styles: target.styles ?? [], annotations });
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
