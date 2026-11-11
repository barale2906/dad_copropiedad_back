# ADR 0013 · Directorio de comercios y servicios (publicidad local, fase 1)

- **Estado:** aceptada
- **Fecha:** 2026-11-10
- **Plan:** `docs/planes/08-publicidad-local.md` (fase 1)

## Contexto
La publicidad local es un negocio **de la plataforma** (decisión del 2026-11-10): la venden y
la cobran sus dueños; las PH no venden pauta, no reciben ingresos ni la configuran. La fase 1
es un directorio de comercios y servicios (tiendas, técnicos, costureras, contadores…) que da
valor aunque no haya pauta paga. El portal del anunciante y su verificación previa son la
fase 3, bloqueada por la revisión legal del contrato con las PH.

## Decisión
- **Datos de plataforma**: `categorias_publicidad` (árbol de dos niveles con sinónimos y
  `restringida`), `anunciantes` (sobre una organización tipo `anunciante`, ubicación y estado de
  verificación) y `fichas_directorio` + `ficha_categorias` (cobertura, contacto, destacada).
  RLS `solo_plataforma`; el catálogo de categorías se lee desde cualquier transacción y solo lo
  escribe la plataforma.
- **Alta por la plataforma** (`plataforma.directorio.gestionar`, en `superusuario`, `comercial`
  y `moderador_publicidad`): el anunciante nace `verificado` porque la plataforma lo revisa al
  darlo de alta. La organización se reutiliza por NIT. Suspender, rechazar o reactivar es de la
  moderación (`plataforma.publicidad.moderar`) con `EstadoVerificacion::transicionar` y motivo.
- **Cobertura sin datos de personas** (`domain::directorio::cubre`): `nacional`, `municipio`
  (mismo código DANE) o `radio` (haversine ≤ km desde el anunciante; si alguna parte no tiene
  coordenadas, se compara el municipio). La función SQL `anunciante_cubre` aplica la misma
  regla y una prueba compara la distancia de SQL con la del dominio.
- **La copropiedad ve el directorio por `fichas_directorio_copropiedad`** (SECURITY DEFINER,
  tenant de `app.tenant_id`): anunciante verificado, ficha publicada, cobertura que alcanza a
  la PH y ninguna categoría restringida; filtros por anunciante, categoría o rama, texto
  normalizado (nombre, descripción, categorías y sinónimos en ambos sentidos: «busco
  costurera» encuentra «costura») y destacadas. Paginación por `id`.
- **Directorio en todas las PH; la pauta solo con el módulo**: el directorio básico funciona
  sin módulo. Una ficha destacada (`destacada_hasta`) solo sale marcada y en
  `/directorio/destacadas` donde la plataforma habilitó `Modulo::Publicidad`.
- **Categorías restringidas** (licores, apuestas, medicamentos con fórmula) nunca entran a una
  ficha: `validar_categorias` en el dominio y un trigger en la BD.
- **Reseñas por copropiedad** (`resenas`, `reportes_resena`, RLS por tenant): solo de cuentas
  con rol `propietario` o `residente` **derivado** (titularidad o residencia) vigente, además
  del permiso `directorio.resenar`; una por cuenta y anunciante en cada PH (volver a calificar
  la reemplaza). Se publican de inmediato y **sin autor**; el texto solo se ve en la misma PH y
  el promedio suma las publicadas de todas. Un residente las reporta (`directorio.ver`); la
  moderación ve la cola de todas las PH (`resenas_reportadas`, solo en plataforma) y oculta con
  motivo o mantiene, actuando en la PH de la reseña y auditado a su nombre.
- **Permisos de copropiedad**: `directorio.ver` (administrador, asistente, consejo,
  propietario, residente) y `directorio.resenar` (administrador, propietario, residente; el
  administrador sin residencia ni titularidad igual no califica).

## Consecuencias
- Logos y documentos del anunciante quedan para la fase 3: `archivos` es por copropiedad y
  hará falta un almacén de plataforma (misma metodología del ADR 0003).
- Teléfono, WhatsApp, correo y dirección de un anunciante persona natural son datos
  personales que él decide publicar; la plataforma debe registrar esa autorización al darlo de
  alta (se formaliza con el portal y la revisión legal de la fase 3).
- La búsqueda por sinónimos ya sirve a «Buscar un proveedor» en el chat (fase 2), que
  reutilizará `fichas_directorio_copropiedad`.
- La cola de moderación y las reseñas ocultas no se borran; la purga y la retención se
  definirán con las de `solicitudes_proveedor` (fase 2).
