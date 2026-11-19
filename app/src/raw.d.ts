// Text files imported whole by Vite (`import text from "./file.md?raw"`), as in the tests.
declare module "*?raw" {
  const text: string;
  export default text;
}

// Vite: every file matching a pattern, each imported whole (as in the tests).
interface ImportMeta {
  glob(pattern: string | string[], options: { query: "?raw"; import: "default"; eager: true }): Record<string, string>;
}
