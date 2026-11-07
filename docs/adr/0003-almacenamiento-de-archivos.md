# ADR 0003 · Archivos en el disco del VPS, listos para migrar a S3

- **Estado:** aceptada
- **Fecha:** 2026-11-06

## Contexto
Varios módulos guardan archivos: fotos de tickets, reglamento de propiedad horizontal, actas,
contratos, carné de vacunas de mascotas, creativos de publicidad. Al inicio basta el disco del
VPS (con su copia de seguridad), pero con el crecimiento conviene un almacenamiento compatible
con S3. El cambio debe ser sencillo y no tocar los módulos.

## Decisión
1. **Un puerto** `AlmacenArchivos` (`guardar`, `obtener`, `eliminar`, `url_temporal`). Ningún
   módulo toca rutas de disco ni SDK de S3.
2. **Clave lógica** `{tenant}/{aa}/{bb}/{archivo_id}` (`domain::ClaveArchivo`), igual en disco y en
   S3. `aa/bb` son los últimos dígitos del id y reparten los archivos en carpetas pequeñas. La
   clave solo se construye a partir de ids, así que no admite `..` ni texto del usuario.
3. **La fila sabe dónde están los bytes**: `archivos.almacen` (`local` | `s3`). Nunca se guardan
   rutas absolutas del servidor.
4. **Un adaptador por almacén** en `infra-archivos`; `ARCHIVOS_ALMACEN` elige cuál usa la API.
   Hoy existe `local` (`ARCHIVOS_DIR`).
5. **Descargas con enlace temporal firmado**: local = endpoint `/api/v1/archivos/descarga` con
   HMAC-SHA256 (`ARCHIVOS_SECRETO_ENLACES`) y vencimiento de 5 minutos; S3 = URL prefirmada. El
   permiso se verifica antes de generar el enlace.
6. **Tipo verificado por contenido** (*magic bytes*), lista blanca PDF, JPEG, PNG, WebP y HEIC,
   límite de tamaño por tipo y SHA-256 de cada archivo.

### Cómo migrar a S3
1. Implementar el adaptador `s3` del puerto (mismas pruebas que el local).
2. Ejecutar la migración por copropiedad con `casos_uso::archivos::migrar_lote(origen=local,
   destino=s3)` hasta que no quede nada: copia, vuelve a leer la copia, verifica el SHA-256 y solo
   entonces cambia `almacen`. Los archivos con error se quedan en `local` y se reintentan.
3. Cambiar `ARCHIVOS_ALMACEN=s3`. Si quedara algún archivo en `local`, la API lo reporta en lugar
   de servir algo equivocado.
4. Tras verificar, borrar el directorio local (la migración nunca borra el origen).

## Consecuencias
- Cambiar de almacén es configuración más un comando, sin tocar los módulos ni los clientes.
- Mientras el almacén sea local, la API sirve los bytes y el disco del VPS debe estar en la copia
  de seguridad. Rotar `ARCHIVOS_SECRETO_ENLACES` solo invalida enlaces vigentes (minutos).
- Pendiente: el endpoint de subida necesita la sesión y los permisos del plan 02; el borrado de
  archivos llegará con las reglas de retención de datos personales; el antivirus (ClamAV) es
  opcional.
