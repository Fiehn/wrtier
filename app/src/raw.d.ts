// Text files imported whole by Vite (`import text from "./file.md?raw"`), as in the tests.
declare module "*?raw" {
  const text: string;
  export default text;
}
