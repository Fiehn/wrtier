// Style: Inclusion titles
// Each Work included whole starts on a new page under its title, with its author's name below
// when that isn't this Work's author. Keep it below the Styles that style Chapter headings.

#show <inclusion-title>: it => {
  pagebreak(weak: true)
  v(20%)
  align(center, text(size: 1.6em, weight: "regular", it.body))
  v(2em)
}
#show <inclusion-author>: it => {
  align(center, text(style: "italic", it.value))
  v(2em)
}
