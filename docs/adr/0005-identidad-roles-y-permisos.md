# ADR 0005 · Identidad global, roles por ámbito y sesiones opacas

- **Estado:** aceptada
- **Fecha:** 2026-11-06

## Contexto
Una misma persona puede ser administradora en una PH, propietaria en otra y contadora en una
tercera; también hay personal de la plataforma (soporte, comercial) y un superusuario. Se vende
por suscripción y hay apps móviles además de la web.

## Decisión
- **Cuenta global** (`cuentas`): se inicia sesión con **usuario o correo** y contraseña
  (Argon2id). El correo se verifica con un código; con él se recupera la contraseña.
- **Dos ámbitos de roles y permisos**: plataforma y copropiedad. El catálogo de permisos vive
  en el código (`domain::Permiso`) y se sincroniza por migración (una prueba compara ambos).
  Roles **plantilla** del sistema (no editables) y roles **propios** (de una PH, o de plataforma
  creados por el superusuario). Un rol solo lleva permisos de su ámbito (FK compuestas en la BD).
- **Asignaciones con vigencia y origen** (`manual`, `contrato_servicio`, `propiedad`,
  `residencia`, `asamblea`): las derivadas nacen y vencen con el hecho que las origina.
- **Superusuario**: todos los permisos de ambos ámbitos en todas las PH, siempre auditado; nunca
  se queda la plataforma sin uno. Cada PH conserva al menos un administrador.
- **No escalamiento**: nadie crea, edita, asigna ni revoca un rol con permisos que no tiene.
- **Tokens opacos** (no JWT): acceso de 30 min y renovación rotativa hasta el vencimiento
  absoluto (30 días; 12 h en equipo compartido, p. ej. portería). Solo se guardan hashes; si un
  token de renovación ya usado vuelve a llegar, se revoca la sesión entera.
- **Segundo factor TOTP**: opcional para todos; obligatorio para operar la plataforma
  (`PLATAFORMA_EXIGE_SEGUNDO_FACTOR`, `true` por defecto).
- **Dos tipos de transacción**: de copropiedad (`app.tenant_id`, RLS por tenant) y de plataforma
  (`app.plataforma = on`). Una transacción de copropiedad no ve cuentas ni sesiones; lo que
  cruza ámbitos va por funciones `SECURITY DEFINER` (`membresias_de_cuenta`, `buscar_cuenta`,
  `buscar_organizacion_por_nit`, `crear_organizacion`).
- **Organizaciones compartidas** por NIT (con dígito de verificación); la reclamación de una
  ficha siempre la revisa soporte.

## Consecuencias
- Cambiar de PH no exige otra autenticación: la PH va en la ruta y el extractor `EnCopropiedad`
  carga los permisos de la cuenta allí (404 si no tiene ningún rol).
- Revocar es inmediato (cerrar sesión, bloquear la cuenta, terminar un contrato).
- Pendientes: límite de peticiones por IP en `/auth`, passkeys, notificación al titular de
  inicios de sesión nuevos y los roles derivados de titularidad y residencia (planes 03 y 07).
