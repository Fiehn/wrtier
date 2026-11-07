// Story element: Dedication
// Written as: #dedication[For M.]
// Shown as: block
// .docx: centred paragraph
// The dedication, alone on its page.

#let dedication(body) = {
  pagebreak(weak: true)
  v(30%)
  [#block(width: 100%, align(center, emph(body))) <dedication>]
  pagebreak(weak: true)
}
