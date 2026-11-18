import rulesDoc from "../../docs/dialogue-rules.md?raw";
import { describe, expect, it } from "vitest";
import { checkDialogue, defaultOptions, DialogueChecker, type Edit, type Finding, type Options } from "./dialogue";

const rules = (text: string, opts?: Options) => checkDialogue(text, opts).map((f) => f.rule);
const one = (text: string, opts?: Options): Finding => {
  const found = checkDialogue(text, opts);
  expect(found.map((f) => f.rule)).toHaveLength(1);
  return found[0];
};
/** The text after accepting suggestion `n` of `f`. */
function accept(text: string, f: Finding, n = 0): string {
  const edits: Edit[] = [...f.suggestions[n].edits].sort((a, b) => b.from - a.from);
  for (const e of edits) text = text.slice(0, e.from) + e.insert + text.slice(e.to);
  return text;
}
const marked = (text: string, f: Finding) => text.slice(f.from, f.to);

describe("the rules doc", () => {
  it("breaks no rule at all", () => {
    expect(checkDialogue(rulesDoc)).toEqual([]);
  });
  it("did, with `--` for the interruption (an en dash in Typst)", () => {
    const doc = rulesDoc.replace('prior---"', 'prior--"');
    expect(rules(doc)).toEqual(["interruption-dash"]);
  });
});

describe("a period before a tag", () => {
  it("becomes a comma", () => {
    const text = '"Hi." she said.';
    const f = one(text);
    expect([f.rule, f.level, marked(text, f), f.verb]).toEqual(["comma-before-tag", "rule", ".", "said"]);
    expect(accept(text, f)).toBe('"Hi," she said.');
    expect(f.suggestions[0].label).toBe('"Hi," she said.');
  });
  it("and a capital pronoun goes lowercase with it, as a Query: it may be a new sentence", () => {
    const text = '"Hi." She said.';
    const f = one(text);
    expect(f.level).toBe("query");
    expect(accept(text, f)).toBe('"Hi," she said.');
    expect(one('"Hello." The man said nothing.').level).toBe("query");
  });
  it("and dialogue with no punctuation before its tag gets the comma", () => {
    const text = 'Obligatory text here, "hey dudes" He said.';
    const f = one(text);
    expect([f.rule, f.level, marked(text, f), f.verb]).toEqual(["comma-before-tag", "rule", '"', "said"]);
    expect(accept(text, f)).toBe('Obligatory text here, "hey dudes," he said.');
    expect(accept('"Hi" she said.', one('"Hi" she said.'))).toBe('"Hi," she said.');
    expect(accept('I saw Anna.\n\n"Hi" Anna said.', one('I saw Anna.\n\n"Hi" Anna said.'))).toBe('I saw Anna.\n\n"Hi," Anna said.');
  });
  it("and an Action beat after dialogue with no punctuation is a sentence of its own", () => {
    const text = '"Hi" she nodded.';
    const f = one(text);
    expect([f.rule, f.level]).toEqual(["action-as-tag", "query"]);
    expect(accept(text, f)).toBe('"Hi." She nodded.');
  });
  it("but not before an Action beat", () => {
    expect(rules('"Hi." She nodded.')).toEqual([]);
  });
});

describe("a capital on a tag", () => {
  it("is a Rule break on a pronoun or a determiner", () => {
    const text = '"Hi," She said.';
    const f = one(text);
    expect([f.rule, f.level, marked(text, f)]).toEqual(["capital-tag", "rule", "She"]);
    expect(accept(text, f)).toBe('"Hi," she said.');
    expect(accept('"Hi," The man said.', one('"Hi," The man said.'))).toBe('"Hi," the man said.');
  });
  it("is fine on a name the Work has mid-sentence somewhere", () => {
    expect(rules('I saw Anna there.\n\n"Hi," Anna said.')).toEqual([]);
    expect(rules('"Hi," I said.')).toEqual([]);
  });
  it("is a Query on a capital the Work never has mid-sentence", () => {
    const f = one('"Hi," Mother said.');
    expect([f.rule, f.level]).toEqual(["capital-tag", "query"]);
  });
  it("after a question or exclamation mark is a rule of its own", () => {
    const text = '"Stop!" He shouted.';
    const f = one(text);
    expect([f.rule, f.level]).toEqual(["capital-tag-after-mark", "rule"]);
    expect(accept(text, f)).toBe('"Stop!" he shouted.');
    expect(rules('"Stop!" he shouted.')).toEqual([]);
  });
  it("on the verb, when it comes first", () => {
    const text = '"Hi," Said the man.';
    expect(accept(text, one(text))).toBe('"Hi," said the man.');
  });
});

describe("a tag before dialogue", () => {
  it("ends with a comma", () => {
    const text = 'She said "Hi."';
    const f = one(text);
    expect([f.rule, marked(text, f)]).toEqual(["tag-before-dialogue", "said"]);
    expect(accept(text, f)).toBe('She said, "Hi."');
  });
  it("and the dialogue starts with a capital", () => {
    expect(accept('She said "hi."', one('She said "hi."'))).toBe('She said, "Hi."');
    expect(accept('She said, "hi."', one('She said, "hi."'))).toBe('She said, "Hi."');
  });
  it("isn't a scare quote", () => {
    expect(rules('They called it "the Pit" for years.')).toEqual([]);
  });
});

describe("a tag in the middle of a sentence", () => {
  it("is fine either way when the case fits", () => {
    expect(rules('"And if," he said, "both stay lowercase."')).toEqual([]);
    expect(rules('"That is all," she said. "New sentence."')).toEqual([]);
  });
  it("offers both ways when it doesn't", () => {
    const text = '"And if," he said, "Both stay lowercase."';
    const f = one(text);
    expect([f.rule, f.level, marked(text, f)]).toEqual(["mid-sentence-tag", "query", "Both"]);
    expect(accept(text, f, 0)).toBe('"And if," he said, "both stay lowercase."');
    expect(accept(text, f, 1)).toBe('"And if," he said. "Both stay lowercase."');
    const lower = '"That is all," she said. "new sentence."';
    expect(accept(lower, one(lower), 0)).toBe('"That is all," she said. "New sentence."');
  });
});

describe("an Action beat written as a tag", () => {
  it("is a Query offering a sentence of its own", () => {
    const text = '"Hi," she nodded.';
    const f = one(text);
    expect([f.rule, f.level, f.verb]).toEqual(["action-as-tag", "query", "nodded"]);
    expect(accept(text, f)).toBe('"Hi." She nodded.');
  });
  it("goes when the verb is added to the Speech verbs", () => {
    const opts = defaultOptions();
    expect(rules('"Fine," she grumbled.')).toEqual([]); // built in
    expect(rules('"Fine," she grinned.')).toEqual(["action-as-tag"]);
    (opts.speechVerbs as Set<string>).add("grinned");
    expect(rules('"Fine," she grinned.', opts)).toEqual([]);
  });
  it("after a question mark needs its capital", () => {
    const text = '"Why?" she frowned.';
    expect(accept(text, one(text))).toBe('"Why?" She frowned.');
  });
});

describe("punctuation outside the quote", () => {
  it("goes inside", () => {
    const text = '"Hi", she said.';
    const f = one(text);
    expect([f.rule, marked(text, f)]).toEqual(["punctuation-outside", '",']);
    expect(accept(text, f)).toBe('"Hi," she said.');
  });
});

describe("an interruption", () => {
  it("ends with an em dash, not an en dash or a hyphen", () => {
    for (const text of ['"But I--"', '"But I-"']) {
      const f = one(text);
      expect(f.rule).toBe("interruption-dash");
      expect(accept(text, f)).toBe('"But I---"');
      expect(f.suggestions[0].label).toBe('"But I—"');
    }
    expect(rules('"But I---"')).toEqual([]);
  });
});

describe("a quote that doesn't close", () => {
  it("is fine when the next paragraph opens with one", () => {
    expect(rules('"First part.\n\n"Second part."')).toEqual([]);
  });
  it("offers to close it, or to open the next paragraph", () => {
    const text = '"First part.\n\nShe left.';
    const f = one(text);
    expect([f.rule, marked(text, f)]).toEqual(["unclosed-quote", "."]);
    expect(accept(text, f, 0)).toBe('"First part."\n\nShe left.');
    expect(accept(text, f, 1)).toBe('"First part.\n\n"She left.');
  });
});

describe("two speakers in one paragraph", () => {
  it("is a Query offering a new paragraph", () => {
    const text = '"Hi," she said. "Hello," he said.';
    const f = one(text);
    expect([f.rule, f.level, marked(text, f)]).toEqual(["speaker-change", "query", "he"]);
    expect(accept(text, f)).toBe('"Hi," she said.\n\n"Hello," he said.');
  });
  it("isn't the same speaker twice, or two kinds that may be one person", () => {
    expect(rules('"Hi," she said. "Again," she said.')).toEqual([]);
    expect(rules('I saw Anna there.\n\n"Hi," Anna said. "Again," she said.')).toEqual([]);
  });
});

describe("reading through markup", () => {
  it("sees through emphasis, and edits the source", () => {
    const text = '"_Hi_." she said.';
    expect(accept(text, one(text))).toBe('"_Hi_," she said.');
  });
  it("never checks comments, code or Story elements", () => {
    expect(rules('// "Hi." she said.\n\nPlain.')).toEqual([]);
    expect(rules('#epigraph["Hi." she said.]')).toEqual([]);
    expect(rules('`"Hi." she said.`')).toEqual([]);
  });
  it("reads curly quotes too", () => {
    expect(rules("“Hi.” she said.")).toEqual(["comma-before-tag"]);
  });
  it("leaves apostrophes alone", () => {
    expect(rules("\"Don't,\" she said. \"It's 'fine,' she says.\"")).toEqual([]);
  });
});

describe("rules switched off", () => {
  it("give nothing", () => {
    const opts = defaultOptions();
    (opts.rules as Set<string>).delete("comma-before-tag");
    expect(rules('"Hi." she said.', opts)).toEqual([]);
  });
});

describe("checking again as the Work is written", () => {
  const work = ['Rain fell.', '"Hi." she said.', 'I saw Anna there.', '"Hi," Anna said.', '"First part.', 'She left.'];
  const join = (paras: string[]) => paras.join("\n\n");

  it("checks only the paragraph that changed", () => {
    const checker = new DialogueChecker();
    checker.check(join(work));
    expect([checker.readCount, checker.checkCount]).toEqual([6, 6]);
    const edited = [...work];
    edited[1] = '"Hello." she said.';
    expect(checker.check(join(edited))).toEqual(checkDialogue(join(edited)));
    expect([checker.readCount, checker.checkCount]).toEqual([1, 1]);
  });
  it("and keeps the others' findings where they've moved to", () => {
    const checker = new DialogueChecker();
    checker.check(join(work));
    const edited = ["A longer first paragraph than before.", ...work.slice(1)];
    const found = checker.check(join(edited));
    expect(found).toEqual(checkDialogue(join(edited)));
    expect(found.map((f) => f.rule)).toEqual(["comma-before-tag", "unclosed-quote"]);
    expect(accept(join(edited), found[0])).toBe(join(["A longer first paragraph than before.", '"Hi," she said.', ...work.slice(2)]));
    expect(checker.checkCount).toBe(1);
  });
  it("checks a paragraph again when a name it asked about comes or goes elsewhere", () => {
    const checker = new DialogueChecker();
    checker.check(join(work));
    const edited = [...work];
    edited[2] = "I saw her there.";
    const found = checker.check(join(edited));
    expect(found).toEqual(checkDialogue(join(edited)));
    expect(found.map((f) => [f.rule, f.level])).toContainEqual(["capital-tag", "query"]);
    expect(checker.checkCount).toBe(2);
  });
  it("checks a quote that doesn't close again when the next paragraph changes", () => {
    const checker = new DialogueChecker();
    checker.check(join(work));
    const edited = [...work];
    edited[5] = '"Second part."';
    const found = checker.check(join(edited));
    expect(found).toEqual(checkDialogue(join(edited)));
    expect(found.map((f) => f.rule)).toEqual(["comma-before-tag"]);
    expect(checker.checkCount).toBe(2);
  });
  it("gives what a check from scratch gives, edit after edit", () => {
    const checker = new DialogueChecker();
    let text = join(work);
    const edits: [string, string][] = [
      ["Rain fell.", "Rain fell.\n\n\"Stop!\" He shouted."],
      ["She left.", "She left. \"Bye\" she nodded."],
      ["\n\nI saw Anna there.", ""],
      ["\"First part.", "\"First part.\""],
    ];
    for (const [from, to] of edits) {
      text = text.replace(from, to);
      expect(checker.check(text)).toEqual(checkDialogue(text));
    }
  });
});
