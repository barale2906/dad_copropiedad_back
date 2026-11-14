// Documento tabular (libros de propietarios y de residentes, Ley 675 art. 51; representación
// gráfica de los documentos electrónicos). Los datos llegan ya formateados como texto en
// `datos.json` (application::puertos::actas::TablaPdf). Con `qr_matriz`, el QR va en todas las
// páginas con al menos 2 cm de lado (exigencia de la DIAN); con `pie`, esa línea va al pie.

#let d = json("datos.json")
#let matriz = d.at("qr_matriz", default: none)
#let pie = d.at("pie", default: none)

// QR dibujado con cuadros a partir de la matriz de módulos (con su margen silencioso).
#let qr(m, lado) = {
  let n = m.len() + 8
  let t = lado / n
  box(width: lado, height: lado, {
    for (y, fila) in m.enumerate() {
      for (x, oscuro) in fila.enumerate() {
        if oscuro {
          place(dx: (x + 4) * t, dy: (y + 4) * t, rect(width: t, height: t, fill: black, stroke: none))
        }
      }
    }
  })
}

#set document(title: d.titulo)
#set page(
  paper: "us-letter",
  flipped: true,
  margin: (x: 1.5cm, top: if matriz != none { 3.4cm } else { 1.8cm }, bottom: 1.8cm),
  numbering: "1 de 1",
  header: if matriz != none {
    grid(
      columns: (1fr, auto),
      align(left + horizon, text(8pt, fill: gray)[#d.copropiedad · #d.titulo]),
      qr(matriz, 2.4cm),
    )
  } else {
    align(right, text(8pt, fill: gray)[#d.copropiedad · #d.titulo])
  },
  footer: if pie != none {
    grid(
      columns: (1fr, auto),
      text(7pt, fill: gray)[#pie],
      context text(8pt, fill: gray)[#counter(page).display("1 de 1", both: true)],
    )
  } else { auto },
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
