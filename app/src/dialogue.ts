// The Dialogue checker's rules (the v2 plan; the rules and their examples are in
// docs/dialogue-rules.md, which must give no Marks at all). English only.
//
// Pure: a Work's text in, what breaks a rule out, with the edits that would put it right. The
// text is read as it shows: emphasis and strong are seen through, `--`, `---` and `...` read as
// their glyphs, and comments, code, raw, math, labels, links, Story elements and Inclusions are
// hard breaks that are never checked. Every finding keeps its place in the text as typed, so a
// Suggestion edits the real source.
//
// Dialogue is text between double quotes (`"`, `“ ”`) within a paragraph; single quotes are
// quotes inside dialogue, or apostrophes. A Dialogue tag is a Speech verb with its subject next
// to it ("she said", "said the man"); a subject with any other verb after a closing quote is an
// Action beat.

import { BREAK, passagesOf, readPara, type Para } from "./prose";

/** How sure a Mark is (CONTEXT.md): a Mistake (red), a Rule break (yellow) or a Query (blue). */
export type Level = "mistake" | "rule" | "query";

/** One edit to the text as typed. */
export type Edit = { from: number; to: number; insert: string };

/** An edit a Mark offers: what the text reads like after it (`label`), and the edits. */
export type Suggestion = { label: string; edits: Edit[] };

/** Something the Dialogue checker found. `from`..`to`: the characters under the Mark, never empty. */
export type Finding = {
  from: number;
  to: number;
  level: Level;
  rule: RuleId;
  /** One line in plain words: "A tag after dialogue starts in lowercase." */
  why: string;
  suggestions: Suggestion[];
  /** The verb this finding turns on, if any: the Mark offers to add it to the Speech verbs or take it off. */
  verb?: string;
};

/** The rules, by the names they're stored under. Never change one once it has shipped. */
export const RULES = [
  { id: "comma-before-tag", level: "rule", about: "Dialogue before a tag ends with a comma: not a period, and not nothing.", example: "\"Hi,\" she said." },
  { id: "capital-tag", level: "rule", about: "A tag after dialogue starts in lowercase.", example: "\"Hi,\" she said." },
  { id: "capital-tag-after-mark", level: "rule", about: "A tag stays lowercase after a question or exclamation mark.", example: "\"Stop!\" he shouted." },
  { id: "tag-before-dialogue", level: "rule", about: "A tag before dialogue ends with a comma, and the dialogue starts with a capital.", example: "She said, \"Hi.\"" },
  { id: "mid-sentence-tag", level: "query", about: "A tag in the middle of a sentence: after a comma the speech goes on in lowercase, after a period it starts again with a capital.", example: "\"And if,\" he said, \"both stay lowercase.\"" },
  { id: "action-as-tag", level: "query", about: "An Action beat is a sentence of its own, not a tag.", example: "\"Hi.\" She nodded." },
  { id: "punctuation-outside", level: "rule", about: "A comma or period after dialogue goes inside the closing quote.", example: "\"Hi,\" she said." },
  { id: "interruption-dash", level: "rule", about: "Interrupted speech ends with an em dash (---).", example: "\"But I---\"" },
  { id: "unclosed-quote", level: "rule", about: "Dialogue that doesn't close in its paragraph goes on in the next, which opens with a quote.", example: "\"…the end of the paragraph.\n\n\"And on.\"" },
  { id: "speaker-change", level: "query", about: "Each new speaker starts a new paragraph.", example: "\"Hi,\" she said.\n\n\"Hello,\" he said." },
] as const satisfies readonly { id: string; level: Level; about: string; example: string }[];

export type RuleId = (typeof RULES)[number]["id"];

/** Speech verbs as written, past and present (no stemming: "said" and "says" are two entries). */
export const SPEECH_VERBS: readonly string[] = [
  "say", "says", "said", "ask", "asks", "asked", "reply", "replies", "replied", "answer", "answers", "answered",
  "whisper", "whispers", "whispered", "shout", "shouts", "shouted", "yell", "yells", "yelled",
  "scream", "screams", "screamed", "cry", "cries", "cried", "call", "calls", "called",
  "exclaim", "exclaims", "exclaimed", "mutter", "mutters", "muttered", "murmur", "murmurs", "murmured",
  "mumble", "mumbles", "mumbled", "add", "adds", "added", "continue", "continues", "continued",
  "repeat", "repeats", "repeated", "explain", "explains", "explained", "insist", "insists", "insisted",
  "demand", "demands", "demanded", "suggest", "suggests", "suggested", "admit", "admits", "admitted",
  "agree", "agrees", "agreed", "argue", "argues", "argued", "protest", "protests", "protested",
  "snap", "snaps", "snapped", "growl", "growls", "growled", "hiss", "hisses", "hissed",
  "stammer", "stammers", "stammered", "stutter", "stutters", "stuttered", "breathe", "breathes", "breathed",
  "tell", "tells", "told", "wonder", "wonders", "wondered", "offer", "offers", "offered",
  "interrupt", "interrupts", "interrupted", "correct", "corrects", "corrected", "announce", "announces", "announced",
  "declare", "declares", "declared", "warn", "warns", "warned", "order", "orders", "ordered",
  "plead", "pleads", "pleaded", "beg", "begs", "begged", "promise", "promises", "promised",
  "observe", "observes", "observed", "note", "notes", "noted", "remark", "remarks", "remarked",
  "respond", "responds", "responded", "retort", "retorts", "retorted", "roar", "roars", "roared",
  "bellow", "bellows", "bellowed", "shriek", "shrieks", "shrieked", "groan", "groans", "groaned",
  "moan", "moans", "moaned", "grumble", "grumbles", "grumbled", "sob", "sobs", "sobbed",
  "drawl", "drawls", "drawled", "gasp", "gasps", "gasped", "concede", "concedes", "conceded",
  "confess", "confesses", "confessed", "prompt", "prompts", "prompted", "inquire", "inquires", "inquired",
  "enquire", "enquires", "enquired", "remind", "reminds", "reminded", "muse", "muses", "mused",
  "venture", "ventures", "ventured", "whimper", "whimpers", "whimpered", "sing", "sings", "sang",
  "chime", "chimes", "chimed", "begin", "begins", "began",
];

/** Words that are never a name, so a capital on them after dialogue is a Rule break. ("I" is left out.) */
const NEVER_NAMES = new Set([
  "he", "she", "they", "we", "you", "it", "him", "her", "them", "us", "me",
  "his", "hers", "their", "theirs", "our", "ours", "your", "yours", "its", "my", "mine",
  "the", "a", "an", "this", "that", "these", "those", "there", "then", "someone", "somebody",
  "everyone", "everybody", "nobody", "one", "both", "each", "another", "some", "all",
]);
const PRONOUNS = new Set(["he", "she", "they", "we", "you", "it", "i"]);
const DETERMINERS = new Set(["the", "a", "an", "his", "her", "their", "my", "our", "your", "its", "this", "that"]);
/** Common irregular past forms, so "she took" reads as an Action beat. (Regular ones end in -ed.) */
const IRREGULAR_PAST = new Set([
  "took", "gave", "went", "ran", "sat", "stood", "threw", "made", "held", "shook", "grew", "felt",
  "kept", "left", "put", "drew", "hid", "rose", "saw", "bit", "broke", "came", "caught", "fell",
  "found", "got", "hung", "knew", "led", "let", "lay", "lit", "lost", "met", "paid", "rode", "slid",
  "spun", "stuck", "struck", "swung", "thought", "tore", "woke", "wore", "won", "wrote", "spat",
  "shut", "set", "cut", "hit", "turned", "bent", "swept", "crept", "slept", "wept", "leapt", "knelt",
]);

/** What the checker is told: the Speech verbs in use, the rules switched on. */
export type Options = { speechVerbs: ReadonlySet<string>; rules: ReadonlySet<RuleId> };

export const defaultOptions = (): Options => ({
  speechVerbs: new Set(SPEECH_VERBS),
  rules: new Set(RULES.map((r) => r.id)),
});

// ---- Words ----

type Word = { at: number; end: number; text: string };

const isUpper = (c: string | undefined) => c !== undefined && c !== c.toLowerCase() && c === c.toUpperCase();
const isLower = (c: string | undefined) => c !== undefined && c !== c.toUpperCase() && c === c.toLowerCase();
const WORD = /\p{L}[\p{L}'’-]*/gu;

/** The words of `text` from `from` to `to`. */
function wordsIn(text: string, from: number, to: number): Word[] {
  const out: Word[] = [];
  WORD.lastIndex = from;
  for (let m = WORD.exec(text); m && m.index < to; m = WORD.exec(text)) {
    out.push({ at: m.index, end: Math.min(m.index + m[0].length, to), text: m[0] });
  }
  return out;
}

const OPEN_DOUBLE = "“";
const CLOSE_DOUBLE = "”";
/** Ends a sentence. */
const SENTENCE_END = /[.!?…]/;
/** What can end the words of dialogue just before its closing quote. */
const DIALOGUE_END = /[.,!?…—–-]/;

/** A quote in a paragraph: where it opens and closes (null: it runs into the next paragraph). */
type Quote = { open: number; close: number | null };

/** The paragraph's quotes, in order. A straight quote opens after a space, a bracket or the start, as Typst's do. */
function quotesIn(text: string): Quote[] {
  const quotes: Quote[] = [];
  let open: number | null = null;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    let opening: boolean;
    if (c === OPEN_DOUBLE) opening = true;
    else if (c === CLOSE_DOUBLE) opening = false;
    else if (c === '"') opening = i === 0 || /[\s([{\u0000]/.test(text[i - 1]);
    else continue;
    if (opening) {
      if (open !== null) quotes.push({ open, close: null }); // never closed: left as it is
      open = i;
    } else if (open !== null) {
      quotes.push({ open, close: i });
      open = null;
    }
  }
  if (open !== null) quotes.push({ open, close: null });
  return quotes;
}

/** The last non-space character inside a quote, if any. */
function lastInside(text: string, q: Quote): number | null {
  for (let i = (q.close ?? text.length) - 1; i > q.open; i--) if (!/\s/.test(text[i])) return i;
  return null;
}

/** The first word inside a quote. */
function firstInside(text: string, q: Quote): Word | null {
  const end = q.close ?? text.length;
  const w = wordsIn(text, q.open + 1, end)[0];
  if (!w) return null;
  // Only if nothing but spaces, an ellipsis or a dash comes before it.
  return /^[\s…—–-]*$/.test(text.slice(q.open + 1, w.at)) ? w : null;
}

/** Who a tag or an Action beat is about. */
type Subject = { words: Word[]; kind: "pronoun" | "name" | "phrase"; key: string };

/** A Dialogue tag or an Action beat next to a quote. */
type Clause = {
  kind: "tag" | "beat";
  /** Its first word, and its verb. */
  first: Word;
  verb: Word;
  subject: Subject;
  /** Where its words stop (a comma, a period…, or the next quote), and the character there. */
  end: number;
  endChar: string;
};

/** A subject made of `words` (one to three), or null if they don't look like one. */
function subjectOf(words: Word[]): Subject | null {
  if (words.length === 0 || words.length > 3) return null;
  const first = words[0].text;
  const lower = first.toLowerCase();
  const key = words.map((w) => w.text.toLowerCase()).join(" ");
  if (words.length === 1 && PRONOUNS.has(lower)) return { words, kind: "pronoun", key };
  if (DETERMINERS.has(lower) && words.length >= 2) return { words, kind: "phrase", key };
  if (words.every((w) => isUpper(w.text[0])) && !NEVER_NAMES.has(lower)) return { words, kind: "name", key };
  return null;
}

/** Ends a clause next to a quote. */
const CLAUSE_STOP = /[.,;:!?…—–"“”\u0000\n]/;

/** Looks like a verb in the past: "nodded", "took". */
const pastLike = (w: string) => /[a-z]ed$/i.test(w) || IRREGULAR_PAST.has(w.toLowerCase());

/** The Work's names, as far as a check asks about them. */
type Names = { has(word: string): boolean };

class Checker {
  constructor(readonly opts: Options, readonly names: Names) {}

  speech(w: string): boolean {
    return this.opts.speechVerbs.has(w.toLowerCase());
  }

  /** The words right after a closing quote, if they're a Dialogue tag or an Action beat. */
  after(text: string, close: number): Clause | null {
    let end = close + 1;
    while (end < text.length && !CLAUSE_STOP.test(text[end])) end++;
    const words = wordsIn(text, close + 1, end);
    if (words.length < 2) return null;
    if (!/^\s*$/.test(text.slice(close + 1, words[0].at))) return null;
    const clause = (kind: Clause["kind"], verb: Word, subject: Subject): Clause =>
      ({ kind, first: words[0], verb, subject, end, endChar: text[end] ?? "" });
    // "said the man", "asked Anna"
    if (this.speech(words[0].text)) {
      const s = subjectOf(words.slice(1, 2)) ?? subjectOf(words.slice(1, 3)) ?? subjectOf(words.slice(1, 4));
      return s ? clause("tag", words[0], s) : null;
    }
    // "she said", "the man said", "Anna quietly said"
    for (let k = 1; k <= Math.min(3, words.length - 1); k++) {
      let subjectWords = words.slice(0, k);
      if (k >= 2 && /ly$/.test(subjectWords[k - 1].text) && !isUpper(subjectWords[k - 1].text[0])) subjectWords = subjectWords.slice(0, -1);
      const subject = subjectOf(subjectWords);
      if (!subject) continue;
      const verb = words[k];
      if (this.speech(verb.text)) return clause("tag", verb, subject);
      if (subjectWords.length === k && pastLike(verb.text)) return clause("beat", verb, subject);
    }
    return null;
  }

  /** The words right before an opening quote, if they end in a Dialogue tag ("She said "hi.""). */
  before(text: string, open: number): { verb: Word; last: Word; comma: boolean; afterQuote: boolean; start: number } | null {
    let i = open - 1;
    while (i >= 0 && /[ \t\n]/.test(text[i])) i--;
    const comma = text[i] === "," || text[i] === ":";
    const stop = comma ? i : i + 1;
    let start = stop;
    while (start > 0 && !/[.;!?…"“”\u0000]/.test(text[start - 1]) && !(text[start - 1] === "," && start - 1 !== i)) start--;
    const words = wordsIn(text, start, stop);
    if (words.length < 2) return null;
    const afterQuote = start > 0 && /["”]/.test(text[start - 1]);
    const n = words.length;
    const last = words[n - 1];
    const candidates: number[] = [n - 1];
    if (/ly$/.test(last.text)) candidates.push(n - 2);
    if (n >= 3 && words[n - 2].text.toLowerCase() === "to" && subjectOf([last])) candidates.push(n - 3);
    // The verb last, or followed only by an adverb ("said quietly") or "to" someone ("said to him").
    for (const k of candidates) {
      if (k >= 1 && this.speech(words[k].text)) return { verb: words[k], last, comma, afterQuote, start };
    }
    return null;
  }

  /** Does a capital on `word` after dialogue break the rule (yes), maybe (null), or not (no)? */
  capitalIsWrong(word: string): boolean | null {
    if (word === "I" || word.startsWith("I'") || word.startsWith("I’")) return false;
    const lower = word.toLowerCase();
    if (NEVER_NAMES.has(lower) || this.speech(word)) return true;
    if (this.names.has(word)) return false;
    return null;
  }
}

// ---- Findings ----

/** Builds a finding's Suggestion from edits to the paragraph as it reads. */
type ReadEdit = { at: number; to: number; insert: string; shows?: string };

function toSource(p: Para, e: ReadEdit): Edit {
  if (e.at === e.to) {
    const from = e.at >= p.text.length ? (p.to[p.text.length - 1] ?? p.start) : p.from[e.at];
    return { from, to: from, insert: e.insert };
  }
  return { from: p.from[e.at], to: p.to[e.to - 1], insert: e.insert };
}

/** What the text around the edits reads like after them: `"Hi," she said.` */
function preview(p: Para, edits: ReadEdit[]): string {
  const lo = Math.min(...edits.map((e) => e.at));
  const hi = Math.max(...edits.map((e) => e.to));
  let from = lo;
  let words = 0;
  while (from > 0 && words < 4) {
    from--;
    if (/\s/.test(p.text[from]) && !/\s/.test(p.text[from + 1] ?? "")) words++;
    if (p.text[from] === BREAK || (p.text[from] === "\n" && words > 0)) { from++; break; }
  }
  let to = hi;
  words = 0;
  while (to < p.text.length && words < 4) {
    if (/\s/.test(p.text[to]) && !/\s/.test(p.text[to - 1] ?? "")) words++;
    if (p.text[to] === BREAK || (words > 2 && /[.!?]/.test(p.text[to - 1]))) break;
    to++;
  }
  let out = "";
  let at = from;
  for (const e of [...edits].sort((a, b) => a.at - b.at)) {
    out += p.text.slice(at, e.at) + (e.shows ?? e.insert);
    at = e.to;
  }
  out += p.text.slice(at, to);
  const text = out.replace(/\n\n/g, " ¶ ").replace(/\s+/g, " ").trim();
  return (from > 0 ? "…" : "") + text + (to < p.text.length ? "…" : "");
}

const swapCase = (c: string) => (isUpper(c) ? c.toLowerCase() : c.toUpperCase());

/** Every finding in a Work's text. */
export function checkDialogue(source: string, opts: Options = defaultOptions()): Finding[] {
  return new DialogueChecker(opts).check(source);
}

/** A paragraph as it reads, as if it started the text (how it reads doesn't hang on where it is), and its names. */
type Read = { para: Para; names: Set<string> };

/** What checking a paragraph found, as if it started the text, and what that turned on besides its own text. */
type Checked = {
  findings: Finding[];
  /** The words asked about as names, and whether the Work had them as names then. */
  asked: Map<string, boolean>;
  /** The next paragraph (how far on it starts, and its text), if the check looked at it; null if it didn't. */
  next: string | null;
};

/**
 * Checks a Work again and again as it's written (one per editor): it keeps how each paragraph
 * reads and what was found in it, by its text, so a look after typing reads and checks only the
 * paragraphs that changed. A paragraph is checked again too if what its findings turned on
 * changed: whether a word it asked about is one of the Work's names, or the next paragraph, which
 * a quote that doesn't close looks at.
 */
export class DialogueChecker {
  private reads = new Map<string, Read>();
  private checks = new Map<string, Checked>();
  /** How many paragraphs the last look read, and checked, afresh (for the tests). */
  readCount = 0;
  checkCount = 0;

  constructor(readonly opts: Options = defaultOptions()) {}

  /** Every finding in the Work's text as it is now. */
  check(source: string): Finding[] {
    // Paragraphs, and each Footnote's text as a paragraph of its own (prose.ts).
    const ranges = passagesOf(source);
    const texts = ranges.map((r) => source.slice(r.start, r.end));
    /** The paragraph after passage `i`: the next one that isn't a Footnote; a Footnote has none. */
    const after = (i: number) => {
      if (ranges[i].note) return -1;
      for (let j = i + 1; j < ranges.length; j++) if (!ranges[j].note) return j;
      return -1;
    };
    this.readCount = 0;
    this.checkCount = 0;

    // Only the paragraphs there now are kept, so what's kept never outgrows the Work.
    const reads = new Map<string, Read>();
    const read = (text: string): Read => {
      let r = reads.get(text) ?? this.reads.get(text);
      if (!r) {
        this.readCount++;
        const para = readPara(text, 0, text.length);
        r = { para, names: namesIn(para) };
      }
      reads.set(text, r);
      return r;
    };
    const names = new Set<string>();
    for (const t of texts) for (const n of read(t).names) names.add(n);
    this.reads = reads;

    const checks = new Map<string, Checked>();
    const out: Finding[] = [];
    ranges.forEach((r, i) => {
      const text = texts[i];
      const n = after(i);
      const nextAt = n !== -1 ? ranges[n].start - r.start : -1;
      const nextKey = nextAt === -1 ? "" : `${nextAt}\u0001${texts[n]}`;
      let done = checks.get(text) ?? this.checks.get(text);
      const fresh = done && (done.next === null || done.next === nextKey)
        && [...done.asked].every(([w, was]) => names.has(w) === was);
      if (!done || !fresh) {
        const asked = new Map<string, boolean>();
        const lookup: Names = { has: (w) => { const is = names.has(w); asked.set(w, is); return is; } };
        let next: string | null = null;
        const nextPara = () => {
          next = nextKey;
          return nextAt === -1 ? null : shiftPara(read(texts[n]).para, nextAt);
        };
        this.checkCount++;
        const findings: Finding[] = [];
        checkParagraph(read(text).para, nextPara, new Checker(this.opts, lookup), findings);
        done = { findings, asked, next };
      }
      checks.set(text, done);
      for (const f of done.findings) out.push(shiftFinding(f, r.start));
    });
    this.checks = checks;

    return out
      .filter((f) => this.opts.rules.has(f.rule))
      .sort((a, b) => a.from - b.from || a.to - b.to);
  }
}

function shiftPara(p: Para, by: number): Para {
  return { ...p, from: p.from.map((x) => x + by), to: p.to.map((x) => x + by), start: p.start + by, end: p.end + by };
}

function shiftFinding(f: Finding, by: number): Finding {
  if (by === 0) return f;
  return {
    ...f,
    from: f.from + by,
    to: f.to + by,
    suggestions: f.suggestions.map((s) => ({ ...s, edits: s.edits.map((e) => ({ ...e, from: e.from + by, to: e.to + by })) })),
  };
}

/** The words a paragraph has with a capital where no sentence starts: the Work's names are all its paragraphs' together. */
function namesIn(p: Para): Set<string> {
  const names = new Set<string>();
  for (const w of wordsIn(p.text, 0, p.text.length)) {
    if (!isUpper(w.text[0]) || w.text === "I") continue;
    let j = w.at - 1;
    while (j >= 0 && /\s/.test(p.text[j])) j--;
    if (j < 0) continue;
    if (/[.!?…:;"“”\u0000—–(]/.test(p.text[j])) continue;
    names.add(w.text.replace(/['’]s$/, ""));
  }
  return names;
}

/** `nextPara`: the next paragraph, read only when a quote that doesn't close needs it (null: there's none). */
function checkParagraph(p: Para, nextPara: () => Para | null, c: Checker, out: Finding[]) {
  const text = p.text;
  if (!/["“”]/.test(text)) return;
  const quotes = quotesIn(text);

  const find = (rule: RuleId, level: Level, at: number, to: number, why: string, suggestions: ReadEdit[][], verb?: Word) => {
    out.push({
      from: p.from[at], to: p.to[to - 1], level, rule, why,
      suggestions: suggestions.map((edits) => ({ label: preview(p, edits), edits: edits.map((e) => toSource(p, e)) })),
      ...(verb ? { verb: verb.text.toLowerCase() } : {}),
    });
  };
  const recase = (w: Word): ReadEdit => ({ at: w.at, to: w.at + 1, insert: swapCase(w.text[0]) });

  /** Who speaks each quote, as far as a tag says, for the speaker check. */
  const speakers: { quote: number; subject: Subject; word: Word; start: number }[] = [];

  quotes.forEach((q, k) => {
    // ---- Before the quote: "She said "hi."" ----
    const before = c.before(text, q.open);
    const first = firstInside(text, q);
    if (before) {
      speakers.push({ quote: k, subject: subjectOf([before.last]) ?? { words: [before.verb], kind: "phrase", key: `?${k}` }, word: before.verb, start: before.start });
      const looksSpoken = q.close === null || DIALOGUE_END.test(text[lastInside(text, q) ?? q.open] ?? "");
      if (!before.afterQuote && looksSpoken) {
        if (!before.comma) {
          const edits: ReadEdit[] = [{ at: before.last.end, to: before.last.end, insert: "," }];
          if (first && isLower(first.text[0])) edits.push(recase(first));
          find("tag-before-dialogue", "rule", before.last.at, before.last.end,
            "A tag before dialogue ends with a comma.", [edits], before.verb);
        } else if (first && isLower(first.text[0])) {
          find("tag-before-dialogue", "rule", first.at, first.end,
            "Dialogue after a tag starts with a capital.", [[recase(first)]], before.verb);
        }
      }
    }

    if (q.close === null) {
      // ---- A quote that doesn't close: the next paragraph must open with one ----
      if (k !== quotes.length - 1) return;
      const next = nextPara();
      const nextOpens = next !== null && /^\s*["“]/.test(next.text);
      if (nextOpens) return;
      let last = text.length - 1;
      while (last > q.open && /\s/.test(text[last])) last--;
      if (text[last] === BREAK) return;
      const close: ReadEdit[] = [{ at: last + 1, to: last + 1, insert: '"' }];
      const suggestions = [close];
      const why = next ? "The quote doesn't close, and the next paragraph doesn't open with one." : "The quote never closes.";
      find("unclosed-quote", "rule", last, last + 1, why, suggestions);
      if (next) {
        const firstChar = next.text.search(/\S/);
        if (firstChar !== -1 && next.text[firstChar] !== BREAK) {
          const at = next.from[firstChar];
          out[out.length - 1].suggestions.push({
            label: `Open the next paragraph with a quote: "${next.text.slice(firstChar, firstChar + 24).trim()}…`,
            edits: [{ from: at, to: at, insert: '"' }],
          });
        }
      }
      return;
    }

    const close = q.close;
    const lastAt = lastInside(text, q);
    const last = lastAt === null ? "" : text[lastAt];

    // ---- Punctuation outside the closing quote: `"Hi", she said.` ----
    const outside = text[close + 1];
    if ((outside === "," || outside === ".") && lastAt !== null && !DIALOGUE_END.test(last) && text[close + 2] !== ".") {
      find("punctuation-outside", "rule", close, close + 2,
        `The ${outside === "," ? "comma" : "period"} goes inside the closing quote.`,
        [[{ at: close + 1, to: close + 2, insert: "" }, { at: close, to: close, insert: outside }]]);
    }

    // ---- Interrupted speech: `--"` or `-"` ----
    if (lastAt !== null && lastAt === close - 1 && (last === "–" || last === "-")) {
      find("interruption-dash", "rule", lastAt, lastAt + 1, "Interrupted speech ends with an em dash.",
        [[{ at: lastAt, to: lastAt + 1, insert: "---", shows: "—" }]]);
    }

    const after = c.after(text, close);
    if (!after) return;
    speakers.push({ quote: k, subject: after.subject, word: after.subject.words[0], start: q.open });

    if (after.kind === "tag") {
      const fw = after.first;
      const capital = isUpper(fw.text[0]) ? c.capitalIsWrong(fw.text) : false;
      if (last === ".") {
        // ---- "Hi." she said. A capital ("Hi." She said nothing.) may be a new sentence: only a Query. ----
        const edits: ReadEdit[] = [{ at: lastAt!, to: lastAt! + 1, insert: "," }];
        if (capital === true) edits.push(recase(fw));
        const sure = !isUpper(fw.text[0]);
        find("comma-before-tag", sure ? "rule" : "query", lastAt!, lastAt! + 1,
          sure ? "Dialogue before a tag ends with a comma, not a period: the tag is part of the same sentence."
            : "If this is a tag, the dialogue before it ends with a comma, and the tag starts in lowercase.", [edits], after.verb);
      } else if (lastAt !== null && !DIALOGUE_END.test(last) && !/[;:]/.test(last)) {
        // ---- "hey dudes" He said. No punctuation at all: a comma goes in, and the tag goes lowercase. ----
        const comma: ReadEdit = { at: lastAt + 1, to: lastAt + 1, insert: "," };
        const suggestions = [capital === true ? [comma, recase(fw)] : [comma]];
        if (capital === null) suggestions.push([comma, recase(fw)]);
        find("comma-before-tag", "rule", close, close + 1,
          capital === false || capital === null
            ? "Dialogue before a tag ends with a comma: the tag is part of the same sentence."
            : "Dialogue before a tag ends with a comma, and the tag starts in lowercase: it's part of the same sentence.",
          suggestions, after.verb);
      } else if (capital !== false && /[,?!…—–-]/.test(last)) {
        // ---- "Hi," She said. / "Stop!" He shouted. ----
        const mark = last === "?" || last === "!";
        find(mark ? "capital-tag-after-mark" : "capital-tag", capital ? "rule" : "query", fw.at, fw.end,
          mark ? "A tag stays lowercase after a question or exclamation mark: it's still the same sentence."
            : capital ? "A tag after dialogue starts in lowercase."
            : `A tag after dialogue starts in lowercase, unless “${fw.text}” is a name.`,
          [[recase(fw)]], after.verb);
      }

      // ---- A tag in the middle of a sentence: "And if," he said, "both…" ----
      const q2 = quotes[k + 1];
      if (last === "," && q2 && (after.endChar === "," || after.endChar === ".")
        && /^\s*$/.test(text.slice(after.end + 1, q2.open))) {
        const w = firstInside(text, q2);
        if (w) {
          const tagEnd = after.end;
          if (after.endChar === "," && isUpper(w.text[0]) && w.text !== "I" && !c.names.has(w.text)) {
            find("mid-sentence-tag", "query", w.at, w.end,
              "After a tag ending in a comma the speech goes on in lowercase; if it's a new sentence, end the tag with a period.",
              [[recase(w)], [{ at: tagEnd, to: tagEnd + 1, insert: "." }]], after.verb);
          } else if (after.endChar === "." && isLower(w.text[0])) {
            find("mid-sentence-tag", "query", w.at, w.end,
              "After a tag ending in a period the speech starts a new sentence, with a capital; if it's the same sentence, end the tag with a comma.",
              [[recase(w)], [{ at: tagEnd, to: tagEnd + 1, insert: "," }]], after.verb);
          }
        }
      }
    } else {
      // ---- An Action beat written as a tag: "Hi," she nodded. ----
      const fw = after.first;
      if (last === ",") {
        const edits: ReadEdit[] = [{ at: lastAt!, to: lastAt! + 1, insert: "." }];
        if (isLower(fw.text[0])) edits.push(recase(fw));
        find("action-as-tag", "query", lastAt!, lastAt! + 1,
          `“${after.verb.text}” isn't a Speech verb, so this reads as an Action beat: a sentence of its own.`, [edits], after.verb);
      } else if (lastAt !== null && !DIALOGUE_END.test(last) && !/[;:]/.test(last)) {
        // ---- "Hi" she nodded. No punctuation: a period, and the beat starts a sentence. ----
        const edits: ReadEdit[] = [{ at: lastAt + 1, to: lastAt + 1, insert: "." }];
        if (isLower(fw.text[0])) edits.push(recase(fw));
        find("action-as-tag", "query", close, close + 1,
          `“${after.verb.text}” isn't a Speech verb, so this reads as an Action beat: the dialogue ends with a period, and the beat is a sentence of its own.`, [edits], after.verb);
      } else if ((last === "?" || last === "!") && isLower(fw.text[0])) {
        find("action-as-tag", "query", fw.at, fw.end,
          `“${after.verb.text}” isn't a Speech verb, so this reads as an Action beat: a sentence of its own, with a capital.`, [[recase(fw)]], after.verb);
      }
    }
  });

  // ---- Two speakers in one paragraph ----
  for (let a = 0; a < speakers.length; a++) {
    const s1 = speakers[a];
    const s2 = speakers.slice(a + 1).find((s) => s.quote !== s1.quote && s.subject.kind === s1.subject.kind
      && !s.subject.key.startsWith("?") && !s1.subject.key.startsWith("?") && s.subject.key !== s1.subject.key
      && !(s.subject.kind === "pronoun" && (s.subject.key === "i" || s1.subject.key === "i" || s.subject.key === "they" || s1.subject.key === "they")));
    if (!s2) continue;
    // The new paragraph starts with the sentence the second speaker's quote is in.
    let start = Math.min(s2.start, quotes[s2.quote].open);
    while (start > 0 && !(SENTENCE_END.test(text[start - 1]) || (/["”]/.test(text[start - 1]) && SENTENCE_END.test(text[start - 2] ?? "")))) {
      if (!/\s/.test(text[start - 1])) { start--; continue; }
      start--;
    }
    // `start` is just after the end of the sentence before; skip the space to the new one.
    let ws = start;
    while (ws < text.length && /\s/.test(text[ws])) ws++;
    if (start === 0 || ws === start) break;
    const w = s2.subject.words[0];
    find("speaker-change", "query", w.at, w.end,
      `A new speaker (“${s2.subject.words.map((x) => x.text).join(" ")}”) starts a new paragraph.`,
      [[{ at: start, to: ws, insert: "\n\n", shows: "\n\n" }]]);
    break;
  }
}
