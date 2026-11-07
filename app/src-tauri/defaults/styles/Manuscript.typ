// Style: Manuscript
// Standard manuscript format: US Letter, 1-inch margins, 12 pt, double-spaced, ragged right,
// page numbers at the top right, and every Chapter on a new page.

#set page(paper: "us-letter", margin: 1in, numbering: "1", number-align: right + top)
#set text(font: "Libertinus Serif", size: 12pt, hyphenate: false)
#set par(justify: false, first-line-indent: 0.5in, leading: 1.3em, spacing: 1.3em)
#show heading.where(level: 1): it => {
  pagebreak(weak: true)
  v(30%)
  align(center, text(size: 12pt, weight: "regular", it.body))
  v(2em)
}
