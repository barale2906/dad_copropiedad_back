# ADR 0024 · Facturador de la plataforma: catálogo, suscripciones, cortes y mora

- **Estado:** aceptada
- **Fecha:** 2026-11-16
- **Plan:** `docs/planes/19-facturador-de-la-plataforma.md`

## Contexto
La plataforma vende módulos (ADR 0016) y pauta (ADR 0023), pero no tenía cómo cobrarlos: los
precios, lo contratado por cada entidad, el consumo medido y las facturas vivían fuera del
sistema. El estado `suspendida` de una entidad tampoco tenía efecto. La entidad de la plataforma
(GISLA) ya es una empresa con su propia facturación (ADR 0022).

## Decisión
- **Catálogo comercial de plataforma** (`productos_plataforma`) con **tarifas versionadas**
  (`tarifas_producto`, solo inserción): `plana`, `por_unidad`, `por_paquete` (base + incluidas +
  excedente), `por_consumo` o `unica`. Precios libres, en COP, sin descuentos automáticos; un
  precio pactado por línea exige motivo. El IVA lo pone el ítem de facturación de GISLA.
- **Suscripción por entidad** (una vigente, `ux_suscripcion_vigente`): ciclo de N meses o por
  evento, prueba de 0 a 30 días, plazo de pago (15 días por defecto) y pagador: la misma entidad,
  otra entidad o una organización. Sus líneas son los productos contratados con fechas.
- **Cortes** (`cortes_facturacion`): `borrador → aprobado → facturado → pagado`, o `descartado`.
  El cálculo es puro (`domain::cobro_plataforma::calcular_corte`: prorrateo por días, excedentes,
  redondeo al peso) y la medición entre entidades pasa por `medir_consumo` (`SECURITY DEFINER`,
  solo conteos, guardados una vez por periodo en `mediciones_consumo`). La huella SHA-256 hace
  reproducible el corte. Un corte **igual al anterior cobrado** se aprueba solo.
- **Facturar** = factura de venta **emitida en el tenant de GISLA** con el origen
  `plataforma.corte` (un origen, una factura): el documento electrónico sale por el flujo normal
  de su facturación. Mientras GISLA no factura en el sistema, el corte se marca **facturado por
  fuera** (número y fecha). Los anunciantes (liquidaciones de publicidad) se cobran con el mismo
  flujo: un corte por liquidación, cliente = su organización.
- **Mora** (`accion_de_mora`, worker `worker_cobros` del gateway): aviso al facturar,
  recordatorio 5 días antes del vencimiento, aviso cada 5 días hábiles después y **suspensión**
  a los 30 días de vencida (o antes, a mano). Nunca con una asamblea instalada (también en la
  suspensión manual: 409). Pagar el último corte vencido reactiva la entidad.
- **Suspendida = solo lectura**: al resolver los permisos (`roles::permisos_y_suspension`) se
  dejan las lecturas, `acceso.*` (portería) y `proteccion_datos.*` (derechos de los titulares,
  Ley 1581) (`permisos::permisos_en_suspension`). El superusuario conserva todo. `capacidades`
  trae `entidad_suspendida`.
- Permisos: `plataforma.comercial.gestionar` (catálogo y suscripciones, plantilla `comercial`),
  `plataforma.cobros.aprobar` y `plataforma.cobros.facturar` (plantilla `facturacion`) y
  `suscripcion.ver` en la entidad (administrador): `GET /entidades/{id}/suscripcion` muestra el
  plan, el consumo del periodo en curso y los cortes aprobados en adelante.

## Consecuencias
- Todo el cobro es de plataforma (`solo_plataforma`); la entidad lo ve solo por la API.
- La liquidación de publicidad no cambia de estado al facturarse su corte: el corte es la fuente
  del cobro.
- Facturar y registrar pagos siguen siendo acciones de la consola; la pasarela de pagos queda
  para después.
