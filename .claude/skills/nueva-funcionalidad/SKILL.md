---
name: nueva-funcionalidad
description: Procedimiento para diseñar e implementar una funcionalidad nueva (endpoint, caso de uso, regla de negocio, adaptador o migración) en dad_copropiedad_back cumpliendo AGENTS.md. Úsala antes de escribir código para cualquier funcionalidad nueva o cambio de comportamiento.
---

# Nueva funcionalidad en dad_copropiedad_back

`AGENTS.md` es la fuente única de verdad; esta skill solo ordena los pasos. Léelo primero.

1. **Requisito e invariantes.** Resume qué se pide y qué invariantes de AGENTS.md §9 toca. Si hay
   ambigüedad legal o de negocio, pregunta antes de seguir.
2. **Reutilización (obligatorio).** Busca lo existente antes de escribir:
   `grep -rn "<concepto>" crates apps migrations` y revisa `crates/domain/src`,
   `crates/application/src/{puertos,casos_uso}`, `crates/infra-*`, `crates/shared`, las macros
   `enum_texto!`/`id_entidad!` y `[workspace.dependencies]`. Reporta qué reutilizas o extiendes y por
   qué descartas lo que no.
3. **Diseño por capas** (AGENTS.md §2): `domain` (tipos y reglas) → `application` (puerto + caso de
   uso) → `infra-*` (adaptador) → `apps/*` (endpoint/handler sin lógica). Cambios de esquema:
   `make migracion NOMBRE=<nombre>` con up/down y RLS.
4. **Pruebas** (AGENTS.md §6): de código (unitarias del dominio, caso de uso con dobles, integración
   con `#[sqlx::test]`) y funcionales (flujo completo con `oneshot` sobre el router). Casos de éxito y
   de error, y una prueba por cada invariante afectado.
5. **Documentación** (AGENTS.md §7): `///` en todo lo público, `#[utoipa::path]` y `ToSchema` en cada
   endpoint y DTO, ADR en `docs/adr/` si es una decisión de arquitectura.
6. **Tecnología nueva:** si agregaste una dependencia o un servicio, actualiza AGENTS.md §4 y pasa `make deny`.
7. **Verificación:** `make verificar` y `make doc` deben pasar; si cambiaron consultas, `make sqlx-prepare`.
8. **Cierre:** recorre la definición de terminado (AGENTS.md §11) e informa qué se reutilizó, qué
   pruebas se agregaron y cómo se verificó.
