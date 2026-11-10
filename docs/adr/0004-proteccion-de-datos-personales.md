# ADR 0004 · Protección de datos personales por finalidad (Ley 1581 de 2012)

- **Estado:** aceptada
- **Fecha:** 2026-11-06

## Contexto
El sistema trata datos de propietarios, tenedores, residentes y visitantes. La Ley 1581 de 2012
y el Decreto 1377 de 2013 exigen autorización previa, expresa e informada por finalidad, prueba
de esa autorización, posibilidad de revocarla y respuesta a consultas (10 días hábiles) y
reclamos (15 días hábiles). Cada copropiedad es responsable del tratamiento; la plataforma es
encargada.

## Decisión
- **Política versionada por copropiedad** (`politicas_tratamiento`): inmutable; un cambio es una
  versión nueva con su fecha de vigencia, nunca en el pasado.
- **Autorización por persona y finalidad** (`autorizaciones_tratamiento`): finalidades
  `administracion`, `comunicaciones`, `control_acceso` y `publicidad`. Guarda la versión de la
  política, el medio y la evidencia. Revocar fija `revocada_en`; la fila se conserva como prueba.
  Solo puede haber una vigente por persona y finalidad (índice único parcial).
- Los módulos preguntan `casos_uso::proteccion_datos::esta_autorizado` antes de tratar datos
  para una finalidad que lo exija (comunicaciones, publicidad).
- **Solicitudes del titular** (`solicitudes_titular`) con vencimiento calculado en días hábiles
  de Colombia (`domain::calendario`), contando desde la fecha local de radicación.
- **Columnas personales clasificadas** con `COMMENT ON COLUMN … IS 'dato_personal: …'`
  (o `dato_sensible:`); una prueba falla si una columna conocida pierde la clasificación.
- La auditoría registra ids, finalidades y fechas, nunca los datos personales.

## Consecuencias
- Hay prueba de cada autorización y de su revocación, por versión de la política.
- Actualización 2026-11-09 (plan 09): endpoints del titular y de la administración en
  `/proteccion-datos/…` y `/personas/{persona_id}/…`; quien actúa es el titular sobre sí mismo o
  quien tiene `proteccion_datos.gestionar`. Las solicitudes tienen detalle, trámite, prórroga
  única con motivo y respuesta (texto o archivo) dentro del módulo, sin pasar por tickets; la
  consulta «mis datos» se audita. Los datos de menores y sensibles ya se marcan en el censo
  (ADR 0007). El borrado efectivo tras una supresión lo hace la administración en el censo.
- Nueva columna con datos personales = su `COMMENT ON` en la misma migración y en la prueba.
