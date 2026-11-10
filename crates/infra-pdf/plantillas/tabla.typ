// Documento tabular (libros de propietarios y de residentes, Ley 675 art. 51). Los datos llegan
// ya formateados como texto en `datos.json` (application::puertos::actas::TablaPdf).

#let d = json("datos.json")

#set document(title: d.titulo)
#set page(
  paper: "us-letter",
  flipped: true,
  margin: (x: 1.5cm, y: 1.8cm),
  numbering: "1 de 1",
  header: align(right, text(8pt, fill: gray)[#d.copropiedad · #d.titulo]),
)
#set text(font: "Libertinus Serif", size: 9pt, lang: "es")
#set table(stroke: 0.4pt + gray, inset: 3pt)

#align(center)[
  #text(14pt)[*#d.titulo*] \
  #d.copropiedad \
  #text(fill: gray)[#d.subtitulo]
]

#table(
  columns: d.columnas.len(),
  table.header(..d.columnas.map(c => [*#c*])),
  ..d.filas.flatten().map(c => [#c]),
)

#if d.filas.len() == 0 [_Sin registros a la fecha de corte._]

#v(1em)
#text(8pt, fill: gray)[Generado el #d.generada · #d.filas.len() registros]
