// Style: Chapter openings
// Every Chapter starts on a new page, with its title centred a little way down.

#show heading.where(level: 1): it => {
  pagebreak(weak: true)
  v(20%)
  align(center, text(size: 1.5em, weight: "regular", it.body))
  v(3em)
}
