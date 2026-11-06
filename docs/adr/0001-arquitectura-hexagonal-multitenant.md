# ADR 0001 · Monolito modular hexagonal, multi-tenant con RLS

- **Estado:** aceptada
- **Fecha:** 2026-11-05

## Contexto
DAD_COPROPIEDAD empieza con el módulo de votaciones: unas 4 asambleas simultáneas de hasta 1.000
inmuebles, con votos por web y por WhatsApp. Debe atender varias copropiedades (PH) desde el inicio,
garantizar la integridad y el secreto del voto, y permitir cambiar de proveedor de WhatsApp.

## Decisión
- **Monolito modular** en un workspace de Cargo, con dos binarios (`api` y `gateway`) y una sola base PostgreSQL. Sin microservicios.
- **Arquitectura hexagonal**: `domain` (sin IO) ← `application` (casos de uso y puertos) ← `infra-*` (adaptadores) ← `apps/*` (composición).
- **Multi-tenant** con `tenant_id` en cada tabla y Row-Level Security; la app usa el rol `dad_app` sin `BYPASSRLS`.
- **Outbox** en PostgreSQL en lugar de una cola externa.

## Consecuencias
- Las reglas de negocio se prueban sin base de datos ni red.
- Un error en una consulta no puede mezclar datos de dos copropiedades.
- Las consultas entre tenants exigen funciones `SECURITY DEFINER` explícitas.
- Si la carga lo exige, el worker se puede separar del gateway sin rediseñar.
