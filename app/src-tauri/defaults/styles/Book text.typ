// Style: Book text
// Book typography: justified, hyphenated, paragraphs marked by an indent rather than a gap.

#set text(font: "Libertinus Serif", size: 11pt, hyphenate: true)
#set par(justify: true, first-line-indent: 1.2em, leading: 0.62em, spacing: 0.62em)

// In a .docx: no widows or orphans, and kerning, as Typst does; the rest is measured.
/* .docx
Normal: <w:widowControl/> <w:kern w:val="2"/>
*/
