// Story element: Scene break
// Written as: #scene-break()
// Shown as: chip
// A break between scenes inside a Chapter. Typing *** on an empty line makes one.

#let scene-break() = [#block(width: 100%, above: 1.4em, below: 1.4em, align(center)[\* \* \*]) <scene-break>]

// In a .docx: never the last line on a page, so it is never lost at a page turn.
/* .docx
Scene break: <w:keepNext/> <w:keepLines/>
*/
