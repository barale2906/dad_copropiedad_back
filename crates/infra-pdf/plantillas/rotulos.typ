// Hoja de rótulos genéricos (Code 128) para la mesa de registro: se imprimen en paquetes
// numerados y la mesa liga cada rótulo a un asistente en el momento. Los anchos de barras y
// espacios llegan calculados en `datos.json`.

#let d = json("datos.json")
#let modulo = 0.3mm

#set document(title: "Rótulos — " + d.titulo)
#set page(paper: "us-letter", margin: 1cm)
#set text(font: "DejaVu Sans Mono", size: 9pt)

#let barras(m) = box(inset: (x: 10 * modulo))[
  #stack(dir: ltr, ..m.enumerate().map(((i, w)) => rect(
    width: w * modulo,
    height: 11mm,
    fill: if calc.even(i) { black } else { white },
    stroke: none,
  )))
]

#grid(
  columns: (1fr, 1fr, 1fr),
  row-gutter: 6mm,
  column-gutter: 4mm,
  ..d.rotulos.map(r => box(
    stroke: (paint: gray, thickness: 0.3pt, dash: "dashed"),
    inset: 3mm,
    width: 100%,
  )[
    #align(center)[
      #text(7pt)[#d.titulo] \
      #barras(r.modulos) \
      #text(11pt, weight: "bold")[#r.codigo]
    ]
  ])
)
