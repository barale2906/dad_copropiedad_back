// Acta de la reunión de la asamblea general de propietarios (Ley 675 de 2001, art. 47).
// Los datos llegan calculados en `datos.json` (application::puertos::actas::DatosActa): esta
// plantilla solo da formato. Los decimales llegan como texto (sin pérdida de precisión).

#let d = json("datos.json")
#let o(v) = if v == none { "—" } else { str(v) }

#set document(title: "Acta — " + d.titulo)
#set page(
  paper: "us-letter",
  margin: (x: 2cm, y: 2.2cm),
  numbering: "1 de 1",
  header: align(right, text(8pt, fill: gray)[#d.copropiedad · Acta]),
)
#set text(font: "Libertinus Serif", size: 10pt, lang: "es")
#set par(justify: true)
#set table(stroke: 0.4pt + gray, inset: 4pt)
#show heading.where(level: 1): set text(12pt)
#show heading.where(level: 2): set text(11pt)

#align(center)[
  #text(14pt, weight: "bold")[#d.copropiedad] \
  #if d.nit != none [NIT #d.nit \ ]
  #v(4pt)
  #text(13pt, weight: "bold")[ACTA DE LA #upper(d.titulo)]
]

#v(8pt)
#table(
  columns: (auto, 1fr),
  stroke: none,
  [*Tipo*], [Reunión #d.tipo, #d.convocatoria convocatoria, modalidad #d.modalidad],
  [*Lugar o enlace*], [#o(d.lugar)],
  [*Fecha convocada*], [#d.fecha],
  [*Instalación*], [#o(d.instalada)],
  [*Cierre*], [#o(d.cerrada)],
)

#if d.advertencias.len() > 0 [
  #block(fill: rgb("#fff4e5"), inset: 8pt, radius: 3pt, width: 100%)[
    *Advertencias* \
    #for a in d.advertencias [- #a \ ]
  ]
]

= Orden del día
#for p in d.orden_del_dia [#p \ ]

= Quórum
#table(
  columns: (auto, auto, auto, auto, auto, auto),
  [*Momento*], [*Hora*], [*Coeficiente*], [*%*], [*Bienes*], [*Asistentes*],
  ..d.quorum.map(q => (
    q.momento, q.hora, str(q.coeficiente), str(q.porcentaje_coeficiente) + " %",
    str(q.unidades) + " de " + str(q.unidades_total), str(q.asistentes),
  )).flatten()
)

= Asistencia
#table(
  columns: (1fr, 1fr, auto, 1fr, auto, 1fr),
  [*Asistente*], [*Bienes*], [*Coeficiente*], [*Poderes*], [*Entrada*], [*Salidas*],
  ..d.asistentes.map(a => (
    a.nombre, a.bienes.join(", "), str(a.coeficiente), a.poderes.join(", "), a.entrada,
    a.salidas.join("; "),
  )).flatten()
)

#if d.morosos.len() > 0 [
  *Bienes en el listado de morosos* (efecto: #d.efecto_mora): #d.morosos.join(", ")
]

= Votaciones
#for p in d.preguntas [
  == #p.orden. #p.enunciado
  Tipo: #p.tipo · Voto #p.tipo_voto · Abierta: #o(p.abierta) · Cerrada: #o(p.cerrada) \
  *Decisión:* #p.decision

  #if p.escrutinio != none [
    #let e = p.escrutinio
    Base de cómputo: #e.base_computo. Habilitados: #e.habilitados.coeficiente
    (#e.habilitados.unidades bienes). Votaron: #e.votantes.coeficiente
    (#e.votantes.unidades bienes). Total de la copropiedad: #e.total.coeficiente
    (#e.total.unidades bienes).
    #table(
      columns: (1fr, auto, auto, auto, auto, auto),
      [*Opción*], [*Coeficiente*], [*% coef.*], [*Bienes*], [*% bienes*], [*Cupos*],
      ..e.opciones.map(x => (
        x.texto, str(x.votos.coeficiente), str(x.porcentaje_coeficiente) + " %",
        str(x.votos.unidades), str(x.porcentaje_unidades) + " %", o(x.cupos),
      )).flatten()
    )
  ]
  #if p.elegidos.len() > 0 [*Elegidos:* #p.elegidos.join(", ")]
  #if p.detalle.len() > 0 [
    #text(9pt)[*Detalle nominal*]
    #table(
      columns: (auto, auto, 1fr, auto, auto, auto),
      [*Bien*], [*Coeficiente*], [*Opción*], [*Canal*], [*Hora*], [*Apoderado*],
      ..p.detalle.map(x => (
        x.bien, str(x.coeficiente), x.opcion, x.canal, x.hora,
        if x.apoderado { "sí" } else { "no" },
      )).flatten()
    )
  ]
]

#v(24pt)
#grid(
  columns: (1fr, 1fr),
  gutter: 24pt,
  [#line(length: 80%) Presidente de la asamblea],
  [#line(length: 80%) Secretario de la asamblea],
)
#v(12pt)
#text(8pt, fill: gray)[Acta generada el #d.generada. Las firmas se incorporan en el ejemplar escaneado.]
