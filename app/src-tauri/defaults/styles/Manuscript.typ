// Style: Manuscript
// Standard manuscript format: US Letter, 1-inch margins, 12 pt, double-spaced, ragged right,
// "Surname / Title / page" at the top right, and every Chapter on a new page.

#set page(paper: "us-letter", margin: 1in)
// The surname and title come from the Work's settings; whichever is missing is left out.
#set page(header: context {
  let surname = document.author.at(0, default: "").split(" ").last()
  let parts = (surname, document.title, counter(page).display()).filter(p => p not in (none, ""))
  align(right, parts.join(" / "))
})
#set text(font: "Libertinus Serif", size: 12pt, hyphenate: false)
#set par(justify: false, first-line-indent: 0.5in, leading: 1.3em, spacing: 1.3em)
#show heading.where(level: 1): it => {
  pagebreak(weak: true)
  v(30%)
  align(center, text(size: 12pt, weight: "regular", it.body))
  v(2em)
}
