# ADR 0023 · Publicidad operada por la plataforma, sin módulo y también en las empresas

- **Estado:** aceptada (reemplaza lo que los ADR 0013 y 0015 dicen del módulo `publicidad` y
  del portal del anunciante).
- **Fecha:** 2026-11-16
- **Plan:** `docs/planes/20-solicitudes-frontend.md` (fase 3)

## Contexto
La publicidad es un negocio de la plataforma (ADR 0015), pero dependía de que la plataforma
habilitara el módulo `publicidad` en cada PH (sin él no había destacadas ni piezas), los comercios
operaban su propio portal (`/anunciantes`, con registro y membresía), y el directorio y las piezas
solo existían en las copropiedades. El frontend pidió (solicitudes P1, P2, P4 y P5) que el
personal de la plataforma lo opere todo, que la entidad no controle nada de la publicidad y que
las empresas también reciban el directorio y las piezas.

## Decisión
- **No es un módulo.** Se retira `Modulo::Publicidad`. La pauta (destacadas y piezas) llega a
  toda entidad activa salvo la plataforma y las que la plataforma **excluya**
  (`exclusiones_publicidad`, con motivo; `plataforma.publicidad.gestionar`). `publicidad_habilitada`
  conserva su firma y mira la exclusión.
- **La opera la plataforma.** Se retira el portal del anunciante. Quien tiene
  `plataforma.publicidad.gestionar` carga campañas y piezas de cualquier comercio y ve sus métricas
  y leads (`/plataforma/publicidad/anunciantes/{id}/…`). **Cuatro ojos:** quien creó o editó una
  campaña, o cargó una pieza, no la modera (`editada_por`, `cargado_por`, `revisado_por` y `CHECK`).
- **Cupones sin cuenta.** El comercio los redime con su **enlace** (`X-Comercio`, token guardado
  como hash y cifrado con el `Cofre` para volver a entregarlo).
- **Enlaces estables.** El enlace público del directorio y el de la pantalla del lobby se guardan
  también cifrados: `GET` devuelve el mismo token mientras no se regenere ni se revoque.
- **Empresas.** El directorio, las piezas, sus eventos, cupones y reportes y la autorización de
  publicidad viven en `/entidades/{entidad_id}/…`. La segmentación elige entidades y tipos de
  entidad; los criterios de la PH (tipo, bienes, estrato) solo los cumple una PH. Siguen siendo
  solo de la PH: reseñas, búsqueda «busco…» por WhatsApp, enlace público, pantalla y masivos.
- La autorización sigue siendo **por persona y entidad**; el empleado de una empresa tiene su
  persona desde que se crea su usuario (plan 20, fase 1).

## Consecuencias
- El cifrado de los enlaces usa la clave del Cofre (`FACTURACION_CLAVE_MAESTRA`): sin ella no se
  generan enlaces estables. Los enlaces anteriores a este cambio no se pueden volver a ver: se
  generan de nuevo.
- `/copropiedades/{id}/directorio` (consulta) y `/copropiedades/{id}/publicidad/…` pasan a
  `/entidades/{id}/…`; `/anunciantes/…` deja de existir.
