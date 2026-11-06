# Instrucciones para GitHub Copilot (VS Code)

Los estándares de este repositorio están en [`AGENTS.md`](../AGENTS.md), la **fuente única de verdad**.
Léelo y cúmplelo antes de proponer o generar código. Si hay conflicto, prevalece `AGENTS.md`.

Lo esencial:
- Arquitectura hexagonal: `domain ← application ← infra-* ← apps/*`. La lógica de negocio va solo en `domain`/`application`.
- Antes de escribir código nuevo, busca qué reutilizar en `crates/` (tipos, puertos, casos de uso, macros).
- Toda funcionalidad lleva pruebas de código (unitarias/integración) y funcionales (flujo completo de la API).
- Documenta con rustdoc (`///`) todo lo público y con utoipa (OpenAPI) cada endpoint REST.
- Multi-tenant con RLS: todo acceso a datos va dentro de `transaccion_tenant`. Respeta los invariantes de votaciones (§9).
- `Decimal` para coeficientes (nunca `f64`), sin `unwrap()` fuera de pruebas, nombres de dominio en español.
- Para terminar: `make verificar`. Si incorporas una tecnología, actualiza `AGENTS.md`.
