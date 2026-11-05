import { EditorView, basicSetup } from "codemirror";
import { invoke } from "@tauri-apps/api/core";

new EditorView({
  doc: "The rain had stopped.\n\nType here…",
  extensions: [basicSetup],
  parent: document.getElementById("editor")!,
});


// Same shape as the Rust `Report` struct.
type Report = {
  rows: number;
  integrity: string;
  size: number;
  modified_ms: number;
  last_rows: string[];
};

const pathInput = document.getElementById("db-path") as HTMLInputElement;
const output = document.getElementById("output")!;

// Remember the path on this Device so you only type it once.
pathInput.value = localStorage.getItem("db-path") ?? "";
pathInput.addEventListener("change", () => localStorage.setItem("db-path", pathInput.value));


// The file's size + modified time the last time *we* looked at it.
let lastSeen = "";
const fingerprint = (r: Report) => `${r.size}/${r.modified_ms}`;


function show(r: Report, note: string) {
  output.textContent = [
    note,
    `rows: ${r.rows}   integrity: ${r.integrity}`,
    `size: ${r.size} bytes   modified: ${new Date(r.modified_ms).toLocaleTimeString()}`,
    "newest rows:",
    ...r.last_rows.map((row) => "  " + row),
  ].join("\n");
}

async function call(command: string, args: Record<string, string>, note: string) {
  try {
    const r = await invoke<Report>(command, { path: pathInput.value, ...args });
    show(r, note);
    lastSeen = fingerprint(r);
  } catch (e) {
    output.textContent = "✘ " + e;
  }
}

document.getElementById("write")!.addEventListener("click", () =>
  call("write_row", { text: `row at ${new Date().toLocaleTimeString()}` }, "✔ wrote a row"),
);
document.getElementById("check")!.addEventListener("click", () => call("check", {}, "✔ checked"));

// Every 3 seconds: has someone else (Syncthing) changed the file since we last looked?
setInterval(async () => {
  if (!pathInput.value || !lastSeen) return;
  try {
    const r = await invoke<Report>("check", { path: pathInput.value });
    if (fingerprint(r) !== lastSeen) {
      show(r, "⚠ the file changed underneath us (Syncthing?)");
      lastSeen = fingerprint(r);
    }
  } catch (e) {
    output.textContent = "✘ " + e;
  }
}, 3000);

// --- Spike 0b: Typst timings ---
const tOutput = document.getElementById("t-output")!;
const log = (line: string) => (tOutput.textContent = line + "\n" + tOutput.textContent);

async function typst(command: string, args: Record<string, string> = {}) {
  log(`… ${command}`);
  try {
    log(await invoke<string>(command, args));
  } catch (e) {
    log("✘ " + e);
  }
}

const fontsDir = document.getElementById("fonts-dir") as HTMLInputElement;
document.getElementById("t-setup")!.addEventListener("click", () => typst("typst_setup", { fontsDir: fontsDir.value }));
document.getElementById("t-edit")!.addEventListener("click", () => typst("typst_edit"));
document.getElementById("t-pdf")!.addEventListener("click", () => typst("typst_pdf"));