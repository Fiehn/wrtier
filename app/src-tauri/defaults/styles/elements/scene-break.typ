// Story element: Scene break
// Written as: #scene-break()
// Shown as: chip
// .docx: centred #
// A break between scenes inside a Chapter. Typing *** on an empty line makes one.

#let scene-break() = [#block(width: 100%, above: 1.4em, below: 1.4em, align(center)[\* \* \*]) <scene-break>]
