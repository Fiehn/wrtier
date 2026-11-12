// The Comments tab: the Work's Comments in text order, then the Detached ones, each with its
// Replies; adding a Comment; and replying, editing, resolving and deleting.
//
// It's drawn from the same answer from Rust as the editor's highlights, so the two always agree,
// and drawn again whenever the History changed. A form being filled in survives that.

import { colourOf, type Reviewer } from "./reviewers";
import type { CommentOp, Mark, ReplyMark } from "./sync";
import { keyLabel } from "./keys";

export type CommentsHooks = {
  /** The Work in front, if it's a Work. */
  work(): string | null;
  marks(work: string): Mark[];
  /** The Comments shown on the Work's Inclusions (ADR 0015), by the Work they belong to. */
  includedMarks(work: string): IncludedMarks[];
  /** `work` may be one that isn't open: a Comment shown on an Inclusion belongs to its own Work. */
  act(work: string, op: CommentOp): void;
  /** Adds a Comment on text selected in an Inclusion of `work`: kept in the Work it includes. */
  addIncluded(work: string, target: IncludedTarget, sender: string, body: string): Promise<void>;
  /** Selects part `part` of a Comment's text; returns how many parts there are. */
  show(work: string, comment: string, part: number): number;
  setPending(work: string, range: { from: number; to: number } | null): void;
  pendingRange(work: string): { from: number; to: number } | null;
  reviewers(): Promise<Reviewer[]>;
  /** How the Author is named in this Work (the Preamble's author name, or "Author"). */
  authorName(work: string): Promise<string>;
  /** Asks for a new Reviewer's name and adds them; null if cancelled. */
  newReviewer(): Promise<Reviewer | null>;
  showResolved(): boolean;
  /** The number of open Comments, for the tab's label. */
  counted(open: number): void;
  error(message: string): void;
};

/** An included Work's Comments shown on one of a Work's Inclusions. */
export type IncludedMarks = { work: string; label: string; marks: Mark[] };

/** Text selected in an Inclusion: its character, the selection in its text, what it includes. */
export type IncludedTarget = { at: number; from: number; to: number; quote: string; label: string };

const AUTHOR = "author";
const NEW_REVIEWER = "new";

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (text) e.textContent = text;
  return e;
}

function button(label: string, run: () => void, title = ""): HTMLButtonElement {
  const b = el("button", "link", label);
  b.type = "button";
  if (title) b.title = title;
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    run();
  });
  return b;
}

function when(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

function shorten(text: string, max: number): string {
  const one = text.replace(/\s+/g, " ").trim();
  return one.length > max ? `${one.slice(0, max - 1)}…` : one;
}

/** The sender last picked on this Device. */
function lastSender(): string {
  try {
    return localStorage.getItem("comment-sender") ?? AUTHOR;
  } catch {
    return AUTHOR;
  }
}

function rememberSender(sender: string) {
  try {
    localStorage.setItem("comment-sender", sender);
  } catch {
    // not remembered, then: no harm done
  }
}

/** A form open in a card: its element stays across redraws, so nothing typed is lost. */
type OpenForm = { comment: string; element: HTMLElement };

export class CommentsPanel {
  private focused = new Set<string>();
  /** Which part of each split Comment "Show me" went to last. */
  private parts = new Map<string, number>();
  private form: OpenForm | null = null;
  /** The new Comment being written: its Work and its form. */
  private adding: { work: string; element: HTMLElement } | null = null;

  constructor(
    private readonly list: HTMLElement,
    private readonly newArea: HTMLElement,
    private readonly hooks: CommentsHooks,
  ) {}

  // ---- Senders ----

  /** A sender picker: the Author, the Reviewers not removed, and "New Reviewer…". */
  private async senderPicker(work: string, chosen: string): Promise<HTMLSelectElement> {
    const select = el("select", "comment-sender-picker");
    select.setAttribute("aria-label", "From");
    const fill = async (pick: string) => {
      const [reviewers, author] = await Promise.all([this.hooks.reviewers(), this.hooks.authorName(work)]);
      const options: HTMLOptionElement[] = [];
      const option = (value: string, text: string) => {
        const o = el("option", "", text);
        o.value = value;
        options.push(o);
      };
      option(AUTHOR, `${author} (you)`);
      for (const r of reviewers.filter((r) => !r.removed)) option(`reviewer/${r.id}`, r.role ? `${r.name} (${r.role})` : r.name);
      option(NEW_REVIEWER, "New Reviewer…");
      select.replaceChildren(...options);
      select.value = options.some((o) => o.value === pick && o.value !== NEW_REVIEWER) ? pick : AUTHOR;
    };
    await fill(chosen);
    let previous = select.value;
    select.addEventListener("change", () => {
      if (select.value !== NEW_REVIEWER) {
        previous = select.value;
        return;
      }
      void (async () => {
        const added = await this.hooks.newReviewer().catch((e) => {
          this.hooks.error(String(e));
          return null;
        });
        await fill(added ? `reviewer/${added.id}` : previous);
        previous = select.value;
      })();
    });
    return select;
  }

  /**
   * A small form: an optional sender picker, a text box, and Save / Cancel. Ctrl+Enter saves and
   * Escape cancels. `save` gets the text (and the sender, if there's a picker).
   */
  private textForm(options: {
    initial: string;
    placeholder: string;
    saveLabel: string;
    picker: HTMLSelectElement | null;
    save: (text: string, sender: string | null) => void;
    cancel: () => void;
  }): HTMLElement {
    const form = el("div", "comment-form");
    const box = el("textarea");
    box.rows = 3;
    box.value = options.initial;
    box.placeholder = options.placeholder;
    const save = () => {
      const text = box.value.trim();
      if (!text) {
        box.focus();
        return;
      }
      options.save(text, options.picker?.value ?? null);
    };
    box.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        save();
      } else if (e.key === "Escape") {
        e.preventDefault();
        options.cancel();
      }
    });
    const actions = el("div", "comment-form-actions");
    const saveButton = el("button", "", options.saveLabel);
    saveButton.type = "button";
    saveButton.addEventListener("click", save);
    const cancelButton = el("button", "", "Cancel");
    cancelButton.type = "button";
    cancelButton.addEventListener("click", () => options.cancel());
    actions.append(cancelButton, saveButton);
    if (options.picker) {
      const from = el("label", "comment-from", "From ");
      from.append(options.picker);
      form.append(from);
    }
    form.append(box, actions);
    form.addEventListener("click", (e) => e.stopPropagation());
    queueMicrotask(() => box.focus());
    return form;
  }

  // ---- Adding ----

  /** Starts a new Comment on `range` of the Work: outlined in the text until it's saved or cancelled. */
  async startAdding(work: string, range: { from: number; to: number }) {
    this.cancelAdding();
    this.hooks.setPending(work, range);
    const picker = await this.senderPicker(work, lastSender());
    const element = this.textForm({
      initial: "",
      placeholder: "Write the Comment…",
      saveLabel: "Save",
      picker,
      save: (body, sender) => {
        const at = this.hooks.pendingRange(work);
        if (!at) {
          this.hooks.error("The text this Comment was for has been deleted. Select some text and add it again.");
          return;
        }
        rememberSender(sender!);
        this.hooks.act(work, { op: "comment", from: at.from, to: at.to, sender: sender!, body });
        this.cancelAdding();
      },
      cancel: () => this.cancelAdding(),
    });
    const heading = el("p", "comment-new-heading", "New Comment on the outlined text");
    element.prepend(heading);
    this.adding = { work, element };
    this.newArea.replaceChildren(element);
    this.newArea.hidden = false;
  }

  /**
   * Starts a new Comment on text selected in one of the Work's Inclusions. It's kept in the
   * Work the Inclusion shows (ADR 0015), so the form says which.
   */
  async startAddingIncluded(work: string, target: IncludedTarget) {
    this.cancelAdding();
    const picker = await this.senderPicker(work, lastSender());
    const element = this.textForm({
      initial: "",
      placeholder: "Write the Comment…",
      saveLabel: "Save",
      picker,
      save: (body, sender) => {
        rememberSender(sender!);
        this.cancelAdding();
        this.hooks.addIncluded(work, target, sender!, body).catch((e) => this.hooks.error(String(e)));
      },
      cancel: () => this.cancelAdding(),
    });
    const heading = el("p", "comment-new-heading", `New Comment on “${shorten(target.quote, 60)}”, kept in ${target.label.split(": ")[0]}`);
    element.prepend(heading);
    this.adding = { work, element };
    this.newArea.replaceChildren(element);
    this.newArea.hidden = false;
  }

  private cancelAdding() {
    if (!this.adding) return;
    this.hooks.setPending(this.adding.work, null);
    this.adding = null;
    this.newArea.replaceChildren();
    this.newArea.hidden = true;
  }

  // ---- Showing ----

  /** Highlighted text was clicked: these Comments are marked and scrolled to. */
  focus(ids: string[]) {
    this.focused = new Set(ids);
    this.render();
    this.list.querySelector(".comment-card.focused")?.scrollIntoView({ block: "nearest" });
  }

  /** Another Work is in front, or its Comments changed: draw the list again. */
  render() {
    const work = this.hooks.work();
    if (this.adding && this.adding.work !== work) this.cancelAdding();
    const marks = work ? this.hooks.marks(work) : [];
    const included = work ? this.hooks.includedMarks(work) : [];
    const everyMark = [...marks, ...included.flatMap((g) => g.marks)];
    if (this.form && !everyMark.some((m) => m.id === this.form!.comment)) this.form = null;
    this.hooks.counted(everyMark.filter((m) => !m.resolved).length);
    const shown = marks.filter((m) => this.hooks.showResolved() || !m.resolved);
    const attached = shown.filter((m) => m.ranges.length > 0);
    const detached = shown.filter((m) => m.ranges.length === 0);
    const children: HTMLElement[] = [];
    // Comments on included text belong to the Work it comes from; acting on one acts there.
    const includedShown = included
      .map((g) => ({ ...g, marks: g.marks.filter((m) => this.hooks.showResolved() || !m.resolved) }))
      .filter((g) => g.marks.length > 0);
    if (!work) {
      children.push(el("p", "muted", "Open a Work to see its Comments."));
    } else if (shown.length === 0 && includedShown.length === 0) {
      const resolved = marks.length - shown.length;
      children.push(el("p", "muted", resolved > 0
        ? `No open Comments (${resolved} resolved).`
        : document.body.classList.contains("phone")
          ? "No Comments yet. Select some text, then tap Comment under it."
          : `No Comments yet. Select some text and right-click it${keyLabel("addComment") ? `, or press ${keyLabel("addComment")}` : ""}.`));
    }
    if (work) {
      children.push(...attached.map((m) => this.card(work, m)));
      for (const group of includedShown) {
        children.push(el("h3", "comments-included-heading", `In ${group.label}`));
        children.push(el("p", "muted comments-included-note", `Kept in ${group.work.split("/").pop()}, and shown there too.`));
        children.push(...group.marks.map((m) => this.card(group.work, m)));
      }
      if (detached.length > 0) {
        children.push(el("h3", "comments-detached-heading", "Detached"));
        children.push(el("p", "muted comments-detached-note", "All of their text has been deleted. They're attached again if any of it comes back."));
        children.push(...detached.map((m) => this.card(work, m)));
      }
    }
    this.list.replaceChildren(...children);
  }

  private card(work: string, mark: Mark): HTMLElement {
    const card = el("article", "comment-card");
    card.dataset.id = mark.id;
    card.classList.toggle("resolved", mark.resolved);
    card.classList.toggle("focused", this.focused.has(mark.id));
    card.style.setProperty("--sender", mark.colour === "author" ? "var(--comment-author)" : mark.colour === "unknown" ? "var(--comment-unknown)" : colourOf(mark.colour));

    const header = el("header");
    header.append(el("strong", "comment-name", mark.name), el("span", "comment-time", when(mark.timeMs)));
    if (mark.resolved) header.append(el("span", "comment-state", "Resolved"));
    card.append(header);
    card.append(el("blockquote", "comment-quote", shorten(mark.quote, 120)));

    const editingThis = this.form?.comment === mark.id && this.form.element.dataset.for === mark.id;
    if (editingThis) card.append(this.form!.element);
    else card.append(el("div", "comment-body", mark.body));
    if (mark.otherBody !== null && !editingThis) card.append(this.conflict(work, mark.id, mark.body, mark.otherBody));

    if (mark.replies.length > 0) {
      const replies = el("div", "comment-replies");
      for (const reply of mark.replies) replies.append(this.reply(work, mark.id, reply));
      card.append(replies);
    }

    const actions = el("div", "comment-actions");
    actions.append(
      button("Reply", () => void this.openReply(work, mark.id)),
      button(mark.resolved ? "Reopen" : "Resolve", () => this.hooks.act(work, { op: "resolve", id: mark.id, resolved: !mark.resolved })),
      button("Edit", () => void this.openEdit(work, mark.id, mark.id, mark.body, mark.sender)),
      button("Delete", () => this.hooks.act(work, { op: "deleteComment", id: mark.id }), "Delete it and its Replies (the History keeps it, so it can be restored)"),
    );
    if (mark.ranges.length > 1) {
      actions.append(button(`Show me (${mark.ranges.length} places)`, () => {
        const part = (this.parts.get(mark.id) ?? -1) + 1;
        this.parts.set(mark.id, part);
        this.hooks.show(work, mark.id, part);
      }, "Its text was split by a Move: steps through the places it's in"));
    }
    card.append(actions);
    // A Reply being written goes at the bottom of its Comment.
    if (this.form?.comment === mark.id && this.form.element.dataset.for === `reply:${mark.id}`) card.append(this.form.element);
    // A reply being edited sits in place of the reply (see `reply`).

    // Clicking the card selects its text (the first place, if it's in several). Not its buttons
    // or forms: on the phone, showing the text leaves the Comments screen.
    card.addEventListener("click", (e) => {
      if ((e.target as Element).closest("button, input, textarea, select, a, form")) return;
      if (mark.ranges.length > 0) this.hooks.show(work, mark.id, 0);
    });
    return card;
  }

  private reply(work: string, comment: string, reply: ReplyMark): HTMLElement {
    const row = el("div", "comment-reply");
    row.style.setProperty("--sender", reply.colour === "author" ? "var(--comment-author)" : reply.colour === "unknown" ? "var(--comment-unknown)" : colourOf(reply.colour));
    const header = el("header");
    header.append(el("strong", "comment-name", reply.name), el("span", "comment-time", when(reply.timeMs)));
    row.append(header);
    if (this.form?.comment === comment && this.form.element.dataset.for === reply.id) {
      row.append(this.form.element);
      return row;
    }
    row.append(el("div", "comment-body", reply.body));
    if (reply.otherBody !== null) row.append(this.conflict(work, reply.id, reply.body, reply.otherBody));
    const actions = el("div", "comment-actions");
    actions.append(
      button("Edit", () => void this.openEdit(work, comment, reply.id, reply.body, reply.sender)),
      button("Delete", () => this.hooks.act(work, { op: "deleteComment", id: reply.id }), "Delete this Reply (the History keeps it)"),
    );
    row.append(actions);
    return row;
  }

  /** Two Devices edited the text without seeing each other's edit: this one won; the other can be used instead. */
  private conflict(work: string, id: string, body: string, other: string): HTMLElement {
    const box = el("div", "comment-conflict");
    box.append(el("p", "", "This was changed on two Devices at the same time, and the text above won. The other version:"));
    box.append(el("div", "comment-body", other));
    const actions = el("div", "comment-actions");
    actions.append(
      button("Use the other one", () => this.hooks.act(work, { op: "editComment", id, body: other })),
      button("Keep this one", () => this.hooks.act(work, { op: "editComment", id, body })),
    );
    box.append(actions);
    box.addEventListener("click", (e) => e.stopPropagation());
    return box;
  }

  private closeForm() {
    this.form = null;
    this.render();
  }

  private async openReply(work: string, comment: string) {
    const picker = await this.senderPicker(work, lastSender());
    const element = this.textForm({
      initial: "",
      placeholder: "Reply…",
      saveLabel: "Reply",
      picker,
      save: (body, sender) => {
        rememberSender(sender!);
        this.hooks.act(work, { op: "reply", comment, sender: sender!, body });
        this.closeForm();
      },
      cancel: () => this.closeForm(),
    });
    element.dataset.for = `reply:${comment}`;
    this.form = { comment, element };
    this.render();
  }

  /** Edits a Comment's or Reply's text, and who sent it. */
  private async openEdit(work: string, comment: string, id: string, body: string, sender: string) {
    const picker = await this.senderPicker(work, sender);
    const element = this.textForm({
      initial: body,
      placeholder: "",
      saveLabel: "Save",
      picker,
      save: (text, chosen) => {
        if (text !== body) this.hooks.act(work, { op: "editComment", id, body: text });
        if (chosen && chosen !== sender) this.hooks.act(work, { op: "setCommentSender", id, sender: chosen });
        this.closeForm();
      },
      cancel: () => this.closeForm(),
    });
    element.dataset.for = id;
    this.form = { comment, element };
    this.render();
  }
}
