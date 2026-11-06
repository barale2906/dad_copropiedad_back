# ADR 0002 · Términos de la Ley 675 de 2001 en todo el sistema

- **Estado:** aceptada
- **Fecha:** 2026-11-06

## Contexto
El producto pasa de ser solo de votaciones a gestionar la copropiedad completa: bienes privados,
bienes comunes, censo, reservas, control de acceso, asambleas y publicidad. En el lenguaje
cotidiano los mismos conceptos tienen nombres distintos ("propiedad", "copropiedad", "unidad",
"inmueble", "zona común"), y algunos se usan al revés de como los define la ley. El esquema
inicial llamaba `inmuebles` a lo que la ley llama bienes privados.

## Decisión
- El vocabulario del sistema (código, BD, API, documentación e interfaz) es el de la **Ley 675
  de 2001**, principalmente su art. 3. El glosario vive en `AGENTS.md` §5.
- La unidad con coeficiente se llama **bien privado**: tabla `bienes_privados`, columnas
  `bien_privado_id`, tipo `BienPrivadoId`, error `BienPrivadoYaVoto`.
- La copropiedad (persona jurídica) sigue siendo el tenant: `copropiedades`, `TenantId`.
- Lo que la ley no define usa un término técnico neutro documentado (`agrupacion`, `cuenta`,
  `organizacion`).
- El cambio en el esquema se hace con una migración nueva (`renombrar_inmuebles_a_bienes_privados`),
  sin editar `esquema_inicial`. Una prueba de esquema falla si vuelve a aparecer el término viejo.

## Consecuencias
- Administradores, contadores y abogados reconocen los términos del reglamento y de las actas;
  el código se lee igual que la norma.
- El renombre se hace ahora, antes de que haya datos reales; después costaría una migración de
  datos y cambios en los clientes de la API.
- El ADR 0001 conserva la palabra "inmuebles" porque los ADR no se reescriben.
