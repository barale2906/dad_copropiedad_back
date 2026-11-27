# Índice de endpoints para el frontend

> **Generado** desde `apps/api/openapi.json` con `make guia-endpoints`; no se edita a mano.
> Ante cualquier diferencia manda el contrato. Las convenciones (sesión, errores,
> paginación, archivos…) están en [guia-frontend.md](guia-frontend.md).

697 operaciones. Rutas sin el prefijo `/api/v1`. **Permiso**: el que exige la
operación según su respuesta 403; en cursiva, la regla cuando no es un permiso (ser el
titular, la papeleta, el enlace firmado…); vacío = basta la sesión o la credencial.
**Errores propios**: además de 401 y 404, que imponen la sesión y la entidad. 🔓 = sin
sesión de persona.

## Módulos

- [salud](#salud) (2): Sondas de vida y disponibilidad del servicio
- [archivos](#archivos) (4): Archivos y adjuntos: descarga con enlaces firmados de vida corta
- [autenticacion](#autenticacion) (7): Registro, inicio y cierre de sesión
- [cuenta](#cuenta) (15): La cuenta autenticada
- [roles](#roles) (14): Roles, permisos y asignaciones
- [organizaciones](#organizaciones) (9): Organizaciones compartidas y contratos de servicio
- [copropiedad](#copropiedad) (7): Ficha de la copropiedad: datos legales, cargos vigentes, advertencias y puntos de atención
- [bienes_privados](#bienes-privados) (16): Agrupaciones, bienes privados, anexos, usos exclusivos y titulares
- [coeficientes](#coeficientes) (6): Cuadros de coeficientes versionados (suma exacta = 1)
- [censo](#censo) (23): Censo: residentes, vehículos, mascotas, personas, autoservicio con aprobación y libros (art. 51)
- [reservas](#reservas) (31): Bienes comunes, reservas, citas con la administración, parqueaderos y sorteos (módulo `bienes_comunes`)
- [tickets](#tickets) (14): Tickets: peticiones, quejas, reclamos y solicitudes con plazos en días u horas hábiles, notas internas, reportes de convivencia confidenciales y encuesta (módulo `gestion_administrativa`)
- [conversaciones](#conversaciones) (7): Chat de WhatsApp: preguntas frecuentes y bandeja de conversaciones escaladas a la administración (módulo `gestion_administrativa`)
- [comunicados](#comunicados) (10): Comunicados de la administración: audiencia (toda la PH, propietarios, residentes, torres, bienes o roles), finalidad (Ley 1581), envío inmediato o programado por push, correo y WhatsApp (solo los importantes y a quien no tiene la app), cartelera con lectura y «Enterado» (módulo `gestion_administrativa`)
- [directorio](#directorio) (23): Directorio de comercios y servicios que alcanzan a la copropiedad (por cercanía, municipio o todo el país), con reseñas anónimas de residentes y propietarios; y, en `/plataforma/publicidad`, el catálogo de categorías, los anunciantes, sus fichas y la moderación (plan 08)
- [mantenimiento](#mantenimiento) (22): Inventario de activos con hoja de vida, planes preventivos, órdenes de trabajo y obligaciones periódicas (módulo `gestion_administrativa`)
- [importaciones](#importaciones) (4): Importación masiva desde CSV/XLSX con vista previa
- [asambleas](#asambleas) (52): Asambleas y votaciones (Ley 675, arts. 37–49): convocatoria con padrón y constancias, mesa de registro asistido, poderes, morosos, quórum en vivo, preguntas (sí/no, selección, elección por cociente), voto del asistente por todos sus bienes, acta en PDF, reportes y papeleta virtual
- [acceso](#acceso) (52): Control de acceso (plan 05): visitas con aviso al primer respondiente y escalamiento (2 min a todos los residentes, 5 min llamada), pre-autorizaciones con QR firmado, correspondencia con avisos cada 24 h y entrega con firma o foto, minuta, bitácora, credenciales, autorizaciones, biometría con consentimiento y la API de los puentes (`/api/v1/puentes`)
- [proteccion_datos](#proteccion-datos) (13): Protección de datos personales (Ley 1581 de 2012): política de tratamiento versionada, autorizaciones por finalidad, consultas y reclamos del titular con plazos en días hábiles y prórroga, «mis datos»
- [auditoria](#auditoria) (1): Consulta de la auditoría de la copropiedad (solo lectura; nunca trae datos personales)
- [publicidad](#publicidad) (38): Publicidad de la plataforma (plan 08): portal del anunciante en `/anunciantes`, moderación, tarifas, política y liquidación en `/plataforma/publicidad`, y en la copropiedad las piezas, eventos, cupones y reportes solo para residentes que autorizaron la publicidad. Las copropiedades no venden ni configuran pauta
- [plataforma](#plataforma) (76): Operación de la plataforma (exige segundo factor)
- [contabilidad](#contabilidad) (51): Contabilidad de cualquier entidad: configuración, plan de cuentas (PUC), terceros, periodos, comprobantes de partida doble, reglas, plantillas, reportes y estados financieros (`/entidades/{entidad_id}/contabilidad`)
- [tesoreria](#tesoreria) (42): Tesorería de cualquier entidad: cuentas de tesorería, recibos de caja, egresos con autorización de otra cuenta, facturas de proveedores con retenciones, facturas de venta y cartera, extractos y conciliación bancaria (`/entidades/{entidad_id}/contabilidad/...`). Cada documento se contabiliza con la regla de su evento; anular es revertir
- [cartera](#cartera) (63): Cartera de la propiedad horizontal (módulo `contabilidad_ph`, `/copropiedades/{copropiedad_id}/cartera/...`): conceptos de cobro, políticas de cartera, módulos de contribución, presupuesto y su ejecución, cuotas extraordinarias, liquidación de expensas con cuentas de cobro, recaudo por bien con imputación y pronto pago, notas crédito, cartera inicial, estado de cuenta y cuadre contra el mayor
- [tributario](#tributario) (15): Tributario de cualquier entidad (plan 15): catálogos de IVA, ICA y exógena, perfil tributario versionado que confirma el contador, resoluciones de numeración, documentos electrónicos (factura, nota crédito, documento soporte) con su proveedor, facturas recibidas por XML con eventos RADIAN, preparación de retención, IVA e ICA, información exógena y beneficiarios finales. El sistema prepara; presentar y firmar lo hace el responsable tributario
- [facturacion](#facturacion) (42): Facturación de cualquier entidad, con o sin contabilidad (plan 18): catálogo de productos y servicios, facturas de venta con número interno, pago manual sin tesorería, documentos electrónicos, compras para el documento soporte, facturas recibidas, libros de ventas y compras y paquete de XML y PDF para el contador. Con contabilidad cada documento se contabiliza solo
- [nomina](#nomina) (38): Nómina de cualquier entidad (plan 16): catálogo laboral vigente, empleados (personas de la entidad) con sus contratos laborales versionados, prórroga y terminación, e importación masiva. Los salarios y las cuentas solo se ven con `nomina.salarios.ver`

## salud

Sondas de vida y disponibilidad del servicio

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | 🔓 `/health` | Liveness: el proceso está vivo. No toca dependencias externas. |  |  | 200 `EstadoVivo` |  |
| GET | 🔓 `/ready` | Readiness: el servicio puede atender tráfico (PostgreSQL responde). |  |  | 200 `EstadoListo` | 503 |

## archivos

Archivos y adjuntos: descarga con enlaces firmados de vida corta

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | 🔓 `/archivos/descarga` | Descarga un archivo con un enlace firmado y vigente. | _Enlace con firma inválida o vencido (`enlace_invalido`)_ |  | 200 `application/pdf` | 400, 403, 429 |
| GET | `/entidades/{entidad_id}/archivos` | Archivo de la entidad, más recientes primero. Sin `archivos.personales.ver` no aparecen los personales ni los sensibles. | `archivos.ver` `archivos.personales.ver` |  | 200 `PaginaRespuesta_ArchivoRespuesta` | 403, 422 |
| POST | `/entidades/{entidad_id}/archivos` | Sube un archivo (PDF, JPEG, PNG, WebP o HEIC). El tipo se verifica por el contenido. | `archivos.subir` | `application/octet-stream` | 201 `ArchivoRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/archivos/{archivo_id}/enlace` | Enlace de descarga de un archivo (vence en 5 minutos). Los personales o sensibles exigen además `archivos.personales.ver`. | `archivos.ver` `archivos.personales.ver` |  | 200 `EnlaceRespuesta` | 403 |

## autenticacion

Registro, inicio y cierre de sesión

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| POST | 🔓 `/auth/registro` | Crea una cuenta. Si trae correo, envía un código para verificarlo. Solo con `REGISTRO_ABIERTO=true` (desarrollo): en producción los usuarios los crean la plataforma (`POST /plataforma/cuentas`) o la entidad (`POST /entidades/{entidad_id}/usuarios`). |  | `RegistroPeticion` | 201 `CuentaCreada` | 409, 422, 429 |
| POST | 🔓 `/auth/sesion` | Inicia sesión con usuario o correo y contraseña. Tras 5 fallos seguidos la cuenta se bloquea 15 minutos. |  | `InicioPeticion` | 200 `InicioRespuesta` | 429 |
| DELETE | `/auth/sesion` | Cierra la sesión actual. |  |  | 204  |  |
| POST | 🔓 `/auth/sesion/segundo-factor` | Completa el inicio de sesión con el código de la app autenticadora. |  | `SegundoFactorPeticion` | 200 `SesionRespuesta` | 429 |
| POST | 🔓 `/auth/sesion/renovar` | Cambia el token de renovación por tokens nuevos. Si llega uno ya usado, se cierra la sesión (alguien lo copió). |  | `RenovarPeticion` | 200 `TokensRespuesta` | 429 |
| POST | 🔓 `/auth/contrasena/recuperar` | Envía un código para restablecer la contraseña. Responde igual exista o no la cuenta. |  | `RecuperarPeticion` | 202  | 429 |
| POST | 🔓 `/auth/contrasena/restablecer` | Restablece la contraseña con el código del correo y cierra todas las sesiones. |  | `RestablecerPeticion` | 204  | 422, 429 |

## cuenta

La cuenta autenticada

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/yo` | Perfil de la cuenta autenticada. |  |  | 200 `PerfilRespuesta` |  |
| GET | `/yo/entidades` | Entidades donde la cuenta puede trabajar: el cliente pide en cuál entrar (también llegan al iniciar sesión). Ordenadas por nombre. |  |  | 200 `[EntidadDeLaCuenta]` |  |
| GET | `/yo/tickets` | Solicitudes (tickets) que la cuenta radicó en todas sus copropiedades, las movidas más recientemente primero. El detalle se consulta dentro de cada copropiedad. |  |  | 200 `[TicketDeCuentaRespuesta]` | 422 |
| POST | `/yo/correo/verificar` | Verifica el correo con el código que se envió. |  | `CodigoPeticion` | 204  | 422 |
| POST | `/yo/correo/reenviar` | Vuelve a enviar el código de verificación del correo. |  |  | 202  | 422 |
| PUT | `/yo/contrasena` | Cambia la contraseña y cierra **todas** las sesiones (hay que volver a entrar). |  | `CambioContrasenaPeticion` | 204  | 422 |
| DELETE | `/yo/sesiones` | Cierra la sesión en todos los dispositivos. |  |  | 200 `SesionesCerradas` |  |
| POST | `/yo/segundo-factor` | Empieza a activar el segundo factor (TOTP). Se confirma con un código de la app. |  |  | 200 `InscripcionRespuesta` | 409 |
| POST | `/yo/segundo-factor/confirmar` | Confirma el segundo factor. Desde el próximo inicio de sesión se pedirá el código. |  | `CodigoPeticion` | 204  | 422 |
| GET | `/yo/dispositivos-push` | Dispositivos activos de la cuenta. |  |  | 200 `[DispositivoPushRespuesta]` |  |
| POST | `/yo/dispositivos-push` | Registra (o reactiva) el dispositivo: desde ahí los avisos de portería llegan por la app y no por WhatsApp. |  | `RegistrarDispositivoPeticion` | 201 `DispositivoPushRespuesta` | 422 |
| DELETE | `/yo/dispositivos-push/{dispositivo_id}` | Retira el dispositivo (al cerrar sesión en la app). |  |  | 204  |  |
| GET | `/yo/nomina/desprendibles` | Mis desprendibles en todas las entidades donde trabajo o trabajé (periodos aprobados). |  |  | 200 `[DesprendibleDeCuentaRespuesta]` |  |
| GET | `/yo/nomina/{entidad_id}/desprendibles/{liquidacion_id}` | PDF de uno de mis desprendibles. |  |  | 200 `application/pdf` | 409 |
| GET | `/yo/nomina/{entidad_id}/certificado-laboral` | Mi certificado laboral en una entidad donde trabajo o trabajé (con mi salario actual). |  |  | 200 `application/pdf` |  |

## roles

Roles, permisos y asignaciones

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/permisos` | Catálogo completo de permisos. Requiere sesión. |  |  | 200 `[PermisoRespuesta]` |  |
| GET | `/entidades/{entidad_id}/roles` | Roles de la copropiedad: plantillas del sistema y propios. | `roles.ver` |  | 200 `[RolRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/roles` | Crea un rol propio. Nunca con permisos que quien lo crea no tiene. | `roles.gestionar` | `NuevoRolPeticion` | 201 `RolRespuesta` | 403, 409 |
| PUT | `/entidades/{entidad_id}/roles/{rol_id}/permisos` | Reemplaza los permisos de un rol propio (las plantillas no se editan). |  | `PermisosPeticion` | 200 `RolRespuesta` | 409 |
| GET | `/entidades/{entidad_id}/asignaciones` | Historial de asignaciones (quién tuvo qué rol y cuándo), más recientes primero. |  |  | 200 `[AsignacionRespuesta]` |  |
| POST | `/entidades/{entidad_id}/asignaciones` | Asigna un rol a una cuenta. Nunca un rol con permisos que quien asigna no tiene. | `roles.asignar` | `AsignacionPeticion` | 201 `AsignacionRespuesta` | 403 |
| DELETE | `/entidades/{entidad_id}/asignaciones/{asignacion_id}` | Termina una asignación desde ahora. La copropiedad no se queda sin administrador. |  |  | 204  | 409 |
| GET | `/entidades/{entidad_id}/cuentas` | Busca una cuenta por usuario o correo exactos para asignarle un rol (nunca listados). |  |  | 200 `CuentaEncontrada` |  |
| POST | `/entidades/{entidad_id}/usuarios/importaciones` | Sube y valida una planilla de usuarios (CSV o XLSX): columnas `nombre`, `correo` y `rol` (código), y opcionales `usuario`, `telefono` y `documento`. No crea nada: devuelve la vista previa con los errores por fila; se aplica con `…/usuarios/importaciones/{id}/confirmar`. | `importaciones.ejecutar` `roles.asignar` | `application/octet-stream` | 201 `VistaPreviaUsuariosRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/usuarios` | Usuarios de la entidad con sus roles y si la contraseña está pendiente. | `roles.ver` |  | 200 `PaginaRespuesta_UsuarioRespuesta` | 403 |
| POST | `/entidades/{entidad_id}/usuarios` | Crea un usuario de la entidad con su rol (o asigna el rol a la cuenta que ya tiene ese correo) y le envía el código para definir su contraseña (o un aviso del acceso nuevo). | `roles.asignar` | `NuevoUsuarioPeticion` | 201 `UsuarioCreadoRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/usuarios/{cuenta_id}/codigo-contrasena` | Reenvía el código para definir la contraseña a un usuario que aún no la define. | `roles.asignar` |  | 204  | 403, 422 |
| GET | `/entidades/{entidad_id}/usuarios/importaciones/plantilla` | Plantilla CSV de usuarios con ejemplos. | _No exige permisos: basta un rol en la entidad_ |  | 200 `text/csv` | 403 |
| POST | `/entidades/{entidad_id}/usuarios/importaciones/{importacion_id}/confirmar` | Crea los usuarios de una importación validada en una sola transacción (si una fila choca con otra cuenta, no se crea ninguno y el mensaje dice la fila). | `importaciones.ejecutar` `roles.asignar` |  | 200 `ImportacionRespuesta` | 403, 409 |

## organizaciones

Organizaciones compartidas y contratos de servicio

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| PATCH | `/organizaciones/{organizacion_id}` | Edita la ficha global: soporte; sin reclamar, quien la creó; reclamada, dueños y gestores. | `plataforma.organizaciones.revisar` | `CambiosFichaPeticion` | 204  | 403 |
| POST | `/organizaciones/{organizacion_id}/miembros` | Agrega una cuenta a la organización o cambia su papel. | `plataforma.organizaciones.revisar` | `MiembroPeticion` | 204  | 403 |
| POST | `/organizaciones/{organizacion_id}/reclamaciones` | Pide manejar la ficha de la organización; la revisa soporte. |  | `ReclamacionPeticion` | 201 `ReclamacionRespuesta` | 409 |
| GET | `/copropiedades/{copropiedad_id}/organizaciones` | Busca una organización por NIT exacto, antes de crearla (para reutilizarla). | `organizaciones.vincular` `organizaciones.crear` |  | 200 `FichaOrganizacionRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/organizaciones` | Crea la organización (o devuelve la existente con ese NIT). | `organizaciones.crear` | `NuevaOrganizacionPeticion` | 200 `OrganizacionCreada` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/contratos-servicio` | Contratos de la copropiedad. | `contratos.gestionar` `organizaciones.vincular` |  | 200 `[ContratoRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/contratos-servicio` | Vincula una organización con la copropiedad mediante un contrato de servicio. | `organizaciones.vincular` | `ContratoPeticion` | 201 `ContratoRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/contratos-servicio/{contrato_id}/terminar` | Termina el contrato: vencen los roles del personal derivados de él. | `contratos.gestionar` |  | 200 `ContratoTerminado` | 403 |
| POST | `/copropiedades/{copropiedad_id}/contratos-servicio/{contrato_id}/asignaciones` | Asigna un rol al personal de la organización, derivado del contrato (vence con él). | `roles.asignar` | `AsignacionContratoPeticion` | 201 `AsignacionRespuesta` | 403, 409 |

## copropiedad

Ficha de la copropiedad: datos legales, cargos vigentes, advertencias y puntos de atención

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/copropiedades/{copropiedad_id}` | Ficha de la copropiedad (la ve cualquier rol con `copropiedad.datos.ver`, también los propietarios). | `copropiedad.datos.ver` |  | 200 `FichaRespuesta` | 403 |
| PATCH | `/copropiedades/{copropiedad_id}` | Actualiza los datos legales, de ubicación y de contacto. Cambiar la plantilla no recalcula los códigos ya asignados. | `copropiedad.datos.editar` | `CambiosCopropiedadPeticion` | 200 `DatosCopropiedadRespuesta` | 403, 409, 422 |
| GET | `/copropiedades/{copropiedad_id}/cargos-vigentes` | Administrador, contador, revisor fiscal, consejo y comité de convivencia vigentes. | `copropiedad.datos.ver` |  | 200 `[CargoRespuesta]` | 403 |
| GET | `/copropiedades/{copropiedad_id}/capacidades` | Permisos de la cuenta en la copropiedad y módulos habilitados. Cualquier cuenta con un rol en la copropiedad puede consultarlo (no exige un permiso). | _No exige permisos: basta un rol en la copropiedad_ |  | 200 `CapacidadesRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/puntos-atencion` | Puntos de atención con su horario. | `copropiedad.datos.ver` |  | 200 `[PuntoAtencionRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/puntos-atencion` | Registra un punto de atención (oficina, portería, recepción) con su horario. | `copropiedad.datos.editar` | `NuevoPuntoPeticion` | 201 `PuntoAtencionRespuesta` | 403, 422 |
| PUT | `/copropiedades/{copropiedad_id}/puntos-atencion/{punto_id}/horario` | Reemplaza el horario de un punto de atención. | `copropiedad.datos.editar` | `HorarioDto` | 204  | 403, 422 |

## bienes_privados

Agrupaciones, bienes privados, anexos, usos exclusivos y titulares

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/copropiedades/{copropiedad_id}/agrupaciones` | Árbol de agrupaciones (hasta 3 niveles) con sus bienes privados activos. | `bienes_privados.ver` |  | 200 `ArbolRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/agrupaciones` | Crea un edificio, etapa, bloque… (máximo 3 niveles). | `agrupaciones.editar` | `NuevaAgrupacionPeticion` | 201 `AgrupacionRespuesta` | 403, 409, 422 |
| GET | `/copropiedades/{copropiedad_id}/bienes-privados` | Listado paginado de bienes privados. | `bienes_privados.ver` |  | 200 `PaginaRespuesta_BienPrivadoRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/bienes-privados` | Crea un bien privado. Queda sin coeficiente hasta que se active un cuadro que lo incluya. | `bienes_privados.editar` | `NuevoBienPeticion` | 201 `BienPrivadoRespuesta` | 403, 409, 422 |
| GET | `/copropiedades/{copropiedad_id}/bienes-privados/{bien_privado_id}` | Ficha de un bien privado: coeficiente vigente, anexos, usos exclusivos y titulares. Un titular vigente ve la de su bien. | `bienes_privados.ver` |  | 200 `FichaBienRespuesta` | 403 |
| PATCH | `/copropiedades/{copropiedad_id}/bienes-privados/{bien_privado_id}` | Edita un bien privado. | `bienes_privados.editar` | `CambiosBienPeticion` | 200 `BienPrivadoRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/bienes-privados/{bien_privado_id}/inactivar` | Inactiva un bien privado (conserva su historia; el próximo cuadro ya no lo incluye). | `bienes_privados.editar` |  | 200 `BienPrivadoRespuesta` | 403 |
| PUT | `/copropiedades/{copropiedad_id}/bienes-privados/{bien_privado_id}/principal` | Vincula un parqueadero, depósito o bodega con folio propio a su bien principal (conserva su coeficiente propio). | `bienes_privados.editar` | `VinculoPeticion` | 200 `BienPrivadoRespuesta` | 403, 422 |
| POST | `/copropiedades/{copropiedad_id}/bienes-privados/{bien_privado_id}/usos-exclusivos` | Asigna un bien común en uso exclusivo (art. 22) al bien privado. No tiene coeficiente. | `bienes_privados.editar` | `NuevoUsoPeticion` | 201 `UsoExclusivoRespuesta` | 403, 409 |
| DELETE | `/copropiedades/{copropiedad_id}/bienes-privados/{bien_privado_id}/usos-exclusivos/{uso_id}` | Termina una asignación de uso exclusivo (desde ahora; queda en el historial). | `bienes_privados.editar` |  | 204  | 403 |
| GET | `/copropiedades/{copropiedad_id}/bienes-privados/{bien_privado_id}/titulares` | Historial de titulares del bien privado (vigentes primero). | `titulares.ver` |  | 200 `[TitularRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/bienes-privados/{bien_privado_id}/titulares` | Registra los titulares de un bien privado que aún no tiene. Los que tienen cuenta reciben el rol `propietario`. | `titulares.editar` | `TitularesPeticion` | 201 `[TitularidadRespuesta]` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/bienes-privados/{bien_privado_id}/transferencias` | Transfiere el bien privado (venta, sucesión): termina a los titulares vigentes —y su rol `propietario`— y registra a los nuevos en una sola transacción. | `titulares.editar` | `TitularesPeticion` | 201 `[TitularidadRespuesta]` | 403, 409, 422 |
| PUT | `/copropiedades/{copropiedad_id}/bienes-privados/{bien_privado_id}/representante` | Designa al representante del bien ante la copropiedad entre sus titulares vigentes. | `titulares.editar` | `RepresentantePeticion` | 204  | 403 |
| GET | `/copropiedades/{copropiedad_id}/mis-bienes-privados` | Bienes privados de los que la cuenta autenticada es titular vigente. | _No exige permisos: basta un rol en la copropiedad_ |  | 200 `[BienPrivadoRespuesta]` | 403 |
| GET | `/copropiedades/{copropiedad_id}/mis-residencias` | Bienes privados donde la cuenta autenticada tiene una residencia vigente (también si no es titular: tenedor, familiar…). Sirve para elegir a nombre de qué bien reservar; los bienes de los que es titular están en `/mis-bienes-privados`. | _No exige permisos: basta un rol en la copropiedad_ |  | 200 `[MiResidenciaRespuesta]` | 403 |

## coeficientes

Cuadros de coeficientes versionados (suma exacta = 1)

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/copropiedades/{copropiedad_id}/cuadros-coeficientes` | Cuadros de coeficientes, del más reciente al más antiguo. | `bienes_privados.ver` |  | 200 `[CuadroRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cuadros-coeficientes` | Crea un cuadro borrador (siguiente versión), opcionalmente copiando el vigente. | `bienes_privados.coeficientes.editar` | `NuevoCuadroPeticion` | 201 `CuadroRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/cuadros-coeficientes/{cuadro_id}` | Cuadro con sus coeficientes y su revisión. | `bienes_privados.ver` |  | 200 `DetalleCuadroRespuesta` | 403 |
| PUT | `/copropiedades/{copropiedad_id}/cuadros-coeficientes/{cuadro_id}/coeficientes` | Reemplaza los coeficientes de un borrador. Devuelve la revisión (suma y diferencia). | `bienes_privados.coeficientes.editar` | `CoeficientesPeticion` | 200 `RevisionRespuesta` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/cuadros-coeficientes/{cuadro_id}/validar` | Revisa el cuadro: suma, diferencia frente a 1, faltantes y sobrantes. | `bienes_privados.ver` |  | 200 `RevisionRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cuadros-coeficientes/{cuadro_id}/activar` | Activa el borrador: el vigente pasa a histórico y cada bien privado toma su coeficiente. Exige suma exactamente 1, todos los bienes activos y ninguna reunión instalada. | `bienes_privados.coeficientes.activar` |  | 200 `CuadroRespuesta` | 403, 409, 422 |

## censo

Censo: residentes, vehículos, mascotas, personas, autoservicio con aprobación y libros (art. 51)

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/copropiedades/{copropiedad_id}/residencias` | Residentes (de un bien o de todos). | `censo.ver` |  | 200 `[ResidenciaRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/residencias` | Registra que una persona reside en un bien privado. Tenedores, familiares y propietarios residentes con cuenta reciben el rol `residente`. | `censo.editar` | `NuevaResidenciaPeticion` | 201 `ResidenciaRespuesta` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/residencias/{residencia_id}/terminar` | Termina una residencia (vence su rol `residente` en la misma transacción). | `censo.editar` |  | 204  | 403 |
| GET | `/copropiedades/{copropiedad_id}/vehiculos` | Vehículos (de un bien, por placa o todos). | `censo.ver` |  | 200 `[VehiculoRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/vehiculos` | Registra un vehículo de un bien privado. | `censo.editar` | `NuevoVehiculoPeticion` | 201 `VehiculoRespuesta` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/vehiculos/{vehiculo_id}/retirar` | Retira un vehículo (su placa queda libre). | `censo.editar` |  | 204  | 403 |
| GET | `/copropiedades/{copropiedad_id}/mascotas` | Mascotas (activas, o todas con `historial=true`). | `censo.ver` |  | 200 `[MascotaRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/mascotas` | Registra una mascota de un bien privado. | `censo.editar` | `NuevaMascotaPeticion` | 201 `MascotaRespuesta` | 403 |
| PATCH | `/copropiedades/{copropiedad_id}/mascotas/{mascota_id}` | Actualiza una mascota (vacunas, póliza, estado…). | `censo.editar` | `CambiosMascotaPeticion` | 200 `MascotaRespuesta` | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/cambios-censo` | Propuestas de cambio: todas (con `censo.aprobar`) o las propias (`mias=true`). | `censo.aprobar` |  | 200 `[CambioRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cambios-censo` | Un residente o propietario propone un cambio del censo de su bien privado (queda pendiente de aprobación). | `censo.editar` | `PropuestaPeticion` | 201 `CambioRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cambios-censo/{cambio_id}/aprobar` | Aprueba una propuesta: crea el registro en la misma transacción. | `censo.aprobar` |  | 200 `CambioRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/cambios-censo/{cambio_id}/rechazar` | Rechaza una propuesta con un motivo. | `censo.aprobar` | `RechazoPeticion` | 200 `CambioRespuesta` | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/libros/propietarios` | Libro de propietarios a una fecha de corte (Ley 675, art. 51), en JSON, CSV o PDF. | `censo.exportar` |  | 200 `[FilaPropietarioRespuesta]` | 403 |
| GET | `/copropiedades/{copropiedad_id}/libros/residentes` | Libro de residentes a una fecha de corte (Ley 675, art. 51), en JSON, CSV o PDF. | `censo.exportar` |  | 200 `[FilaResidenteRespuesta]` | 403 |
| GET | `/copropiedades/{copropiedad_id}/censo/alertas` | Vacunas y pólizas vencidas o por vencer (30 días) y residentes sin autorización de datos. | `censo.ver` |  | 200 `AlertasCensoRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/personas` | Personas registradas en la copropiedad (paginado). | `personas.ver` |  | 200 `PaginaRespuesta_PersonaRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/personas/{persona_id}` | Una persona. | `personas.ver` |  | 200 `PersonaRespuesta` | 403 |
| PATCH | `/copropiedades/{copropiedad_id}/personas/{persona_id}` | Actualiza los datos de una persona. | `personas.editar` | `CambiosPersonaPeticion` | 200 `PersonaRespuesta` | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/censo/campanas` | Campañas de actualización, de la más reciente a la más antigua (máximo 100). | `censo.ver` |  | 200 `[ResumenCampanaRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/censo/campanas` | Lanza una campaña: la plantilla `actualizar_datos` llega al responsable de cada bien con residentes (o al representante de los titulares si nadie reside) que tenga teléfono. | `censo.editar` |  | 201 `CampanaCensoRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/censo/campanas/{campana_id}/respuestas` | Respuestas de una campaña; primero las que tienen cambios por atender. | `censo.ver` |  | 200 `[RespuestaCensoRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/censo/campanas/{campana_id}/respuestas/{persona_id}/{bien_privado_id}/atender` | Marca atendida una respuesta con cambios (ya aplicados en el censo). | `censo.editar` |  | 204  | 403 |

## reservas

Bienes comunes, reservas, citas con la administración, parqueaderos y sorteos (módulo `bienes_comunes`)

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/copropiedades/{copropiedad_id}/recursos` | Recursos reservables (bienes comunes, parqueaderos, agenda, herramientas). | `bienes_comunes.ver` |  | 200 `[RecursoRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/recursos` | Crea un bien común, la agenda de la administración o una herramienta prestable. | `bienes_comunes.editar` | `NuevoRecursoPeticion` | 201 `RecursoRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/recursos/{recurso_id}` | Un recurso. | `bienes_comunes.ver` |  | 200 `RecursoRespuesta` | 403 |
| PATCH | `/copropiedades/{copropiedad_id}/recursos/{recurso_id}` | Edita datos, política, horario o detalle de un recurso. | `bienes_comunes.editar` `parqueaderos.asignar` | `CambiosRecursoPeticion` | 200 `RecursoRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/recursos/{recurso_id}/estado` | Cambia el estado del recurso. Cerrarlo (mantenimiento, fuera de servicio, inactivo) cancela las reservas futuras y avisa a los solicitantes. | `bienes_comunes.editar` `parqueaderos.asignar` | `EstadoRecursoPeticion` | 200 `EstadoRecursoRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/recursos/{recurso_id}/disponibilidad` | Franjas libres del recurso (horario − excepciones − reservas − alistamiento). | `bienes_comunes.ver` |  | 200 `[FranjaReservaDto]` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/reservas` | Reservas (todas con `reservas.ver_todas`, o las propias con `mias=true`). | `reservas.ver_todas` |  | 200 `[ReservaRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/reservas` | Solicita una reserva. Con confirmación inmediata queda aprobada; si no, solicitada. | `reservas.crear` | `SolicitudPeticion` | 201 `ReservaRespuesta` | 403, 409, 422 |
| GET | `/copropiedades/{copropiedad_id}/reservas/{reserva_id}` | Una reserva con su entrega y su recibo (propia, o con `reservas.ver_todas`). | `reservas.ver_todas` `reservas.entregas.registrar` |  | 200 `DetalleReservaRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/reservas/{reserva_id}/aprobar` | Aprueba una reserva solicitada (avisa al solicitante). | `reservas.aprobar` |  | 200 `ReservaRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/reservas/{reserva_id}/rechazar` | Rechaza una reserva solicitada. | `reservas.aprobar` | `MotivoReservaPeticion` | 200 `ReservaRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/reservas/{reserva_id}/cancelar` | Cancela una reserva (la propia, con penalidad si es tarde; o cualquiera con `reservas.cancelar_cualquiera`, sin penalidad). | `reservas.cancelar_cualquiera` | `MotivoReservaPeticion` | 200 `ReservaRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/reservas/{reserva_id}/entrega` | Registra la entrega del recurso (aprobada → en curso). | `reservas.entregas.registrar` | `EntregaPeticion` | 200 `ReservaRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/reservas/{reserva_id}/recibo` | Registra el recibo (en curso → finalizada, o con novedad si hay novedad o daños). | `reservas.entregas.registrar` | `EntregaPeticion` | 200 `ReservaRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/reservas/{reserva_id}/no-asistio` | Marca que el solicitante no asistió. | `reservas.entregas.registrar` |  | 200 `ReservaRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/reservas/{reserva_id}/pago` | Registra a mano el pago de una reserva (pagado o exonerado). | `reservas.aprobar` | `PagoPeticion` | 200 `ReservaRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/recursos/{recurso_id}/cobro` | Cómo se cobran las reservas del recurso. | `bienes_comunes.ver` |  | 200 `CobroRecursoDto` | 403 |
| PUT | `/copropiedades/{copropiedad_id}/recursos/{recurso_id}/cobro` | El contador decide cómo se cobra el valor de las reservas del recurso (rige para las que se aprueben desde ahora). | `cartera.configurar` `contabilidad.configurar` | `CobroRecursoDto` | 200 `CobroRecursoDto` | 403, 422 |
| POST | `/copropiedades/{copropiedad_id}/reservas/{reserva_id}/factura` | Factura en borrador por el valor de una reserva de un recurso que se cobra con factura (luego se emite y se cobra como cualquier factura de venta). | `facturacion.facturas.gestionar` |  | 201 `FacturaVentaRespuesta` | 403, 409, 422 |
| GET | `/copropiedades/{copropiedad_id}/restricciones-reserva` | Restricciones de reserva (mora o sanción). | `reservas.restringir` |  | 200 `[RestriccionRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/restricciones-reserva` | Restringe las reservas de un bien privado (marca manual de mora o sanción; no aplica a los bienes comunes esenciales, art. 59). | `reservas.restringir` | `RestriccionPeticion` | 201 `RestriccionRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/restricciones-reserva/{restriccion_id}/levantar` | Levanta una restricción desde ahora. | `reservas.restringir` |  | 204  | 403 |
| PUT | `/copropiedades/{copropiedad_id}/residencias/{residencia_id}/autorizacion-reservas` | El propietario (o la administración) autoriza o retira las reservas de su tenedor, cuando la política del recurso lo exige. | `censo.editar` | `AutorizacionPeticion` | 204  | 403 |
| GET | `/copropiedades/{copropiedad_id}/parqueaderos` | Parqueaderos con su asignación vigente. | `parqueaderos.ver` |  | 200 `[ParqueaderoRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/parqueaderos` | Crea un parqueadero (privado, de uso exclusivo, comunal o de visitantes). | `parqueaderos.asignar` | `NuevoParqueaderoPeticion` | 201 `RecursoRespuesta` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/parqueaderos/{recurso_id}/asignaciones` | Asigna un parqueadero a un bien privado por un periodo. | `parqueaderos.asignar` | `AsignacionParqueaderoPeticion` | 201 `AsignacionParqueaderoRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/asignaciones-parqueadero/{asignacion_id}/liberar` | Libera un parqueadero (termina la asignación desde ahora). | `parqueaderos.asignar` |  | 204  | 403 |
| GET | `/copropiedades/{copropiedad_id}/sorteos` | Sorteos de la copropiedad (públicos para quien ve los bienes comunes). | `bienes_comunes.ver` |  | 200 `[SorteoRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/sorteos` | Crea un sorteo y publica el compromiso de su semilla. | `sorteos.gestionar` | `NuevoSorteoPeticion` | 201 `SorteoRespuesta` | 403, 422 |
| POST | `/copropiedades/{copropiedad_id}/sorteos/{sorteo_id}/inscripciones` | Inscribe un bien privado (lo hace su titular o residente, o la administración). | `sorteos.gestionar` | `InscripcionPeticion` | 204  | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/sorteos/{sorteo_id}/ejecutar` | Ejecuta el sorteo: revela la semilla y asigna los parqueaderos por el periodo. | `sorteos.gestionar` |  | 200 `ResultadoRespuesta` | 403, 409 |

## tickets

Tickets: peticiones, quejas, reclamos y solicitudes con plazos en días u horas hábiles, notas internas, reportes de convivencia confidenciales y encuesta (módulo `gestion_administrativa`)

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/copropiedades/{copropiedad_id}/tickets/configuracion` | Configuración de tickets de la copropiedad. | `tickets.configurar` |  | 200 `ConfiguracionTicketsDto` | 403 |
| PUT | `/copropiedades/{copropiedad_id}/tickets/configuracion` | Reemplaza la configuración (jornada, reapertura y todas las reglas de plazo). | `tickets.configurar` | `ConfiguracionTicketsDto` | 200 `ConfiguracionTicketsDto` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/tickets/categorias` | Categorías de tickets (las inactivas solo para quien configura). | `tickets.crear` |  | 200 `[CategoriaRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/tickets/categorias` | Crea una categoría. | `tickets.configurar` | `CategoriaPeticion` | 201 `CategoriaRespuesta` | 403, 409 |
| PUT | `/copropiedades/{copropiedad_id}/tickets/categorias/{categoria_id}` | Renombra, activa o inactiva una categoría. | `tickets.configurar` | `CategoriaPeticion` | 200 `CategoriaRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/tickets` | Tickets visibles para la cuenta, más recientes primero: todos con `tickets.ver_todos`; los de convivencia con `tickets.convivencia.ver`; los asignados con `tickets.atender`; y los propios. | `tickets.crear` `tickets.ver_todos` `tickets.atender` `tickets.convivencia.ver` |  | 200 `PaginaRespuesta_TicketRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/tickets` | Radica un ticket. Si es urgente avisa de inmediato a quienes tienen `tickets.urgencias.recibir`. | `tickets.crear` | `RadicarPeticion` | 201 `TicketRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/tickets/tablero` | Tablero: pendientes por estado, vencidos, tiempos promedio y satisfacción. | `tickets.ver_todos` |  | 200 `TableroTicketsRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/tickets/{ticket_id}` | Un ticket (sin notas internas para el solicitante). | `tickets.ver_todos` `tickets.convivencia.ver` |  | 200 `DetalleRespuesta` | 403 |
| PATCH | `/copropiedades/{copropiedad_id}/tickets/{ticket_id}` | Reclasifica un ticket; si cambia el tipo o la prioridad se recalcula el vencimiento. | `tickets.asignar` | `ClasificarPeticion` | 200 `TicketRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/tickets/{ticket_id}/asignacion` | Asigna el ticket a una cuenta (con `tickets.atender`) o a una organización con contrato. | `tickets.asignar` | `ResponsableDto` | 200 `TicketRespuesta` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/tickets/{ticket_id}/transiciones` | Cambia el estado según el diagrama y el papel de la cuenta (gestiona, atiende o es el solicitante, que puede cerrar o reabrir dentro del plazo). | `tickets.asignar` `tickets.atender` | `TransicionTicketPeticion` | 200 `TicketRespuesta` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/tickets/{ticket_id}/mensajes` | Agrega un mensaje público o una nota interna. Si el ticket esperaba al solicitante y él responde, vuelve a `en_progreso`. | `tickets.atender` `tickets.asignar` | `MensajePeticion` | 201 `MensajeRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/tickets/{ticket_id}/encuesta` | El solicitante califica la atención (una vez, con el ticket resuelto o cerrado). | `tickets.crear` | `EncuestaPeticion` | 200 `TicketRespuesta` | 403, 409 |

## conversaciones

Chat de WhatsApp: preguntas frecuentes y bandeja de conversaciones escaladas a la administración (módulo `gestion_administrativa`)

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/copropiedades/{copropiedad_id}/preguntas-frecuentes` | Preguntas frecuentes (las inactivas solo para quien las edita). | `copropiedad.datos.ver` `preguntas_frecuentes.editar` |  | 200 `[PreguntaFrecuenteRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/preguntas-frecuentes` | Crea una pregunta frecuente. | `preguntas_frecuentes.editar` | `PreguntaFrecuentePeticion` | 201 `PreguntaFrecuenteRespuesta` | 403, 422 |
| PUT | `/copropiedades/{copropiedad_id}/preguntas-frecuentes/{pregunta_id}` | Reemplaza una pregunta frecuente. | `preguntas_frecuentes.editar` | `PreguntaFrecuentePeticion` | 200 `PreguntaFrecuenteRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/conversaciones` | Bandeja de conversaciones, más recientes primero. | `conversaciones.atender` |  | 200 `PaginaRespuesta_ConversacionRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/conversaciones/{conversacion_id}` | Una conversación con sus mensajes. | `conversaciones.atender` |  | 200 `DetalleConversacionRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/conversaciones/{conversacion_id}/mensajes` | Responde por WhatsApp (dentro de la ventana de 24 h desde el último mensaje del residente). | `conversaciones.atender` | `RespuestaPeticion` | 201 `MensajeConversacionRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/conversaciones/{conversacion_id}/cierre` | Cierra la conversación (el chat vuelve al menú en el siguiente mensaje del residente). | `conversaciones.atender` |  | 200 `ConversacionRespuesta` | 403, 409 |

## comunicados

Comunicados de la administración: audiencia (toda la PH, propietarios, residentes, torres, bienes o roles), finalidad (Ley 1581), envío inmediato o programado por push, correo y WhatsApp (solo los importantes y a quien no tiene la app), cartelera con lectura y «Enterado» (módulo `gestion_administrativa`)

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/copropiedades/{copropiedad_id}/comunicados` | Comunicados de la PH con sus contadores, del más reciente al más antiguo. | `comunicados.gestionar` |  | 200 `PaginaRespuesta_ComunicadoConResumenRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/comunicados` | Crea un comunicado en borrador. | `comunicados.gestionar` | `ComunicadoPeticion` | 201 `ComunicadoRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/comunicados/{comunicado_id}` | Un comunicado con su tablero de entrega y lectura. | `comunicados.gestionar` |  | 200 `ComunicadoConResumenRespuesta` | 403 |
| PUT | `/copropiedades/{copropiedad_id}/comunicados/{comunicado_id}` | Reemplaza el contenido de un borrador. | `comunicados.gestionar` | `ComunicadoPeticion` | 200 `ComunicadoRespuesta` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/comunicados/{comunicado_id}/publicar` | Publica un borrador: sin `programado_para` sale de inmediato (push, correo y, si es importante, WhatsApp a quien no tiene la app). | `comunicados.gestionar` | `PublicarPeticion` | 200 `ComunicadoRespuesta` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/comunicados/{comunicado_id}/retirar` | Devuelve un comunicado programado a borrador. | `comunicados.gestionar` |  | 200 `ComunicadoRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/comunicados/{comunicado_id}/anular` | Anula un borrador o un programado (lo enviado no se anula). | `comunicados.gestionar` |  | 200 `ComunicadoRespuesta` | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/mis-comunicados` | Comunicados recibidos por la persona de la cuenta (vacío si no tiene persona en la PH). | _No exige permisos: basta un rol en la copropiedad; módulo no contratado (`modulo_no_habilitado`)_ |  | 200 `PaginaRespuesta_ComunicadoRecibidoRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/mis-comunicados/{comunicado_id}` | Abre un comunicado recibido: registra la lectura y entrega los adjuntos con enlaces de descarga de 5 minutos. | _No exige permisos (si no se le envió a la persona de la cuenta, 404); módulo no contratado (`modulo_no_habilitado`)_ |  | 200 `ComunicadoAbiertoRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/mis-comunicados/{comunicado_id}/enterado` | Registra el «Enterado» (idempotente). | _No exige permisos (si no se le envió a la persona de la cuenta, 404); módulo no contratado (`modulo_no_habilitado`)_ |  | 200 `ComunicadoRecibidoRespuesta` | 403, 409 |

## directorio

Directorio de comercios y servicios que alcanzan a la copropiedad (por cercanía, municipio o todo el país), con reseñas anónimas de residentes y propietarios; y, en `/plataforma/publicidad`, el catálogo de categorías, los anunciantes, sus fichas y la moderación (plan 08)

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/plataforma/publicidad/categorias` | Todo el catálogo de categorías (incluidas las restringidas e inactivas). | `plataforma.directorio.gestionar` |  | 200 `[CategoriaDirectorioRespuesta]` | 403 |
| PUT | `/plataforma/publicidad/categorias/{codigo}` | Crea o edita una categoría del catálogo. | `plataforma.directorio.gestionar` | `CategoriaDirectorioPeticion` | 200 `CategoriaDirectorioRespuesta` | 403, 422 |
| GET | `/plataforma/publicidad/anunciantes` | Anunciantes de la plataforma, paginados. | `plataforma.directorio.gestionar` `plataforma.publicidad.moderar` |  | 200 `PaginaRespuesta_AnuncianteRespuesta` | 403 |
| POST | `/plataforma/publicidad/anunciantes` | Da de alta un anunciante (queda verificado: la plataforma lo revisa al darlo de alta). | `plataforma.directorio.gestionar` | `NuevoAnunciantePeticion` | 201 `AnuncianteRespuesta` | 403, 409, 422 |
| GET | `/plataforma/publicidad/anunciantes/{anunciante_id}` | Anunciante con su ficha. | `plataforma.directorio.gestionar` `plataforma.publicidad.moderar` |  | 200 `AnuncianteRespuesta` | 403 |
| PUT | `/plataforma/publicidad/anunciantes/{anunciante_id}` | Edita nombre, dirección y ubicación del anunciante. | `plataforma.directorio.gestionar` | `DatosAnunciantePeticion` | 200 `AnuncianteRespuesta` | 403, 422 |
| PUT | `/plataforma/publicidad/anunciantes/{anunciante_id}/ficha` | Crea o reemplaza la ficha del directorio del anunciante. | `plataforma.directorio.gestionar` | `FichaPeticion` | 200 `FichaAnuncianteRespuesta` | 403, 422 |
| POST | `/plataforma/publicidad/anunciantes/{anunciante_id}/estado` | Verifica, rechaza, suspende o reactiva un anunciante. | `plataforma.publicidad.moderar` | `EstadoAnunciantePeticion` | 200 `AnuncianteRespuesta` | 403, 409, 422 |
| GET | `/plataforma/publicidad/moderacion/resenas` | Cola de moderación: reseñas publicadas con reportes sin atender, de todas las copropiedades. | `plataforma.publicidad.moderar` |  | 200 `PaginaRespuesta_ResenaReportadaRespuesta` | 403 |
| POST | `/plataforma/publicidad/moderacion/resenas/{resena_id}` | Atiende los reportes de una reseña: la oculta con motivo o la mantiene. | `plataforma.publicidad.moderar` | `ModeracionPeticion` | 204  | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/directorio/anunciantes/{anunciante_id}/resenas` | Reseñas publicadas del anunciante en esta copropiedad, de la más reciente a la más antigua. | `directorio.ver` |  | 200 `PaginaRespuesta_ResenaRespuesta` | 403 |
| PUT | `/copropiedades/{copropiedad_id}/directorio/anunciantes/{anunciante_id}/resena` | Califica (o vuelve a calificar) a un anunciante. Solo propietarios y residentes vigentes; una reseña por cuenta y anunciante en la copropiedad. | `directorio.resenar` | `ResenaPeticion` | 200 `ResenaRespuesta` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/directorio/resenas/{resena_id}/reportes` | Reporta una reseña publicada para que la modere la plataforma. | `directorio.ver` | `ReporteResenaPeticion` | 200 `ReporteResenaRespuesta` | 403, 422 |
| POST | `/copropiedades/{copropiedad_id}/directorio/busquedas` | Busca proveedores que alcanzan a la copropiedad y registra la búsqueda (el texto se borra a los 90 días). No comparte nada con los proveedores. | `directorio.ver` | `BusquedaPeticion` | 201 `BusquedaRespuesta` | 403, 422 |
| POST | `/copropiedades/{copropiedad_id}/directorio/busquedas/{solicitud_id}/contacto` | Autoriza expresamente que los proveedores sugeridos en la búsqueda lo contacten: solo a ellos y solo para esta solicitud se les comparte su nombre y su celular del censo (Ley 1581). Repetirlo no vuelve a enviar nada. | `directorio.ver` |  | 200 `ContactoAutorizadoRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/directorio/enlace-publico` | El enlace público vigente del directorio (con su token, sin cambiarlo) y el enlace `wa.me` para los grupos. | `directorio.compartir` |  | 200 `EstadoEnlaceRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/directorio/enlace-publico` | Genera el enlace público del directorio para fijarlo en los grupos de WhatsApp de la PH. Si ya había uno, el anterior deja de funcionar. | `directorio.compartir` |  | 201 `EnlaceGeneradoRespuesta` | 403, 422 |
| DELETE | `/copropiedades/{copropiedad_id}/directorio/enlace-publico` | Revoca el enlace público del directorio. | `directorio.compartir` |  | 204  | 403 |
| GET | 🔓 `/copropiedades/{copropiedad_id}/directorio/publico` | Directorio de comercios y servicios de la copropiedad para los grupos de WhatsApp (sin sesión: el token del enlace en `X-Enlace-Directorio` es la credencial). Sin reseñas: solo el promedio. |  |  | 200 `DirectorioPublicoRespuesta` | 429 |
| GET | `/entidades/{entidad_id}/directorio` | Directorio de comercios y servicios que alcanzan a la copropiedad, paginado. | `directorio.ver` |  | 200 `PaginaRespuesta_FichaDirectorioRespuesta` | 403 |
| GET | `/entidades/{entidad_id}/directorio/destacadas` | Fichas destacadas vigentes (hasta 10). Vacío si la copropiedad no tiene el módulo `publicidad`. | `directorio.ver` |  | 200 `[FichaDirectorioRespuesta]` | 403 |
| GET | `/entidades/{entidad_id}/directorio/categorias` | Categorías para filtrar el directorio (activas y no restringidas). | `directorio.ver` |  | 200 `[CategoriaDirectorioRespuesta]` | 403 |
| GET | `/entidades/{entidad_id}/directorio/anunciantes/{anunciante_id}` | Ficha de un anunciante que alcanza a la copropiedad. | `directorio.ver` |  | 200 `FichaDirectorioRespuesta` | 403 |

## mantenimiento

Inventario de activos con hoja de vida, planes preventivos, órdenes de trabajo y obligaciones periódicas (módulo `gestion_administrativa`)

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/copropiedades/{copropiedad_id}/activos` | Inventario de activos (paginado). | `inventario.ver` |  | 200 `PaginaRespuesta_ActivoRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/activos` | Registra un activo (queda `operativo`, con un movimiento de entrada). | `inventario.editar` | `ActivoPeticion` | 201 `ActivoRespuesta` | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/activos/{activo_id}` | Hoja de vida: el activo con sus archivos, movimientos y órdenes de trabajo. | `inventario.ver` |  | 200 `HojaDeVidaRespuesta` | 403 |
| PATCH | `/copropiedades/{copropiedad_id}/activos/{activo_id}` | Edita los datos o el estado operativo de un activo. | `inventario.editar` | `CambiosActivoPeticion` | 200 `ActivoRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/activos/{activo_id}/archivos` | Agrega un archivo (manual, foto, factura, garantía) a la hoja de vida. | `inventario.editar` | `ArchivoActivoDto` | 204  | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/activos/{activo_id}/movimientos` | Registra un movimiento (el traslado cambia la ubicación; la baja es definitiva). | `inventario.editar` | `MovimientoPeticion` | 201 `MovimientoRespuesta` | 403, 409, 422 |
| GET | `/copropiedades/{copropiedad_id}/planes-mantenimiento` | Planes preventivos. | `mantenimiento.planificar` `inventario.ver` |  | 200 `[PlanRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/planes-mantenimiento` | Crea un plan preventivo; el worker genera sus órdenes con la anticipación indicada. | `mantenimiento.planificar` | `PlanPeticion` | 201 `PlanRespuesta` | 403, 422 |
| PUT | `/copropiedades/{copropiedad_id}/planes-mantenimiento/{plan_id}` | Reemplaza un plan preventivo (las órdenes ya generadas no cambian). | `mantenimiento.planificar` | `PlanPeticion` | 200 `PlanRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/ordenes-trabajo` | Órdenes de trabajo, más recientes primero (quien solo ejecuta ve las asignadas). | `mantenimiento.planificar` `mantenimiento.ejecutar` `mantenimiento.verificar` `inventario.ver` |  | 200 `PaginaRespuesta_OrdenRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/ordenes-trabajo` | Crea una orden; si nace de un ticket, el ticket avanza y el solicitante recibe un mensaje. | `mantenimiento.planificar` | `OrdenPeticion` | 201 `OrdenRespuesta` | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/ordenes-trabajo/{orden_id}` | Una orden de trabajo con sus evidencias. | `mantenimiento.planificar` `mantenimiento.verificar` `inventario.ver` `mantenimiento.ejecutar` |  | 200 `DetalleOrdenRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/ordenes-trabajo/{orden_id}/avance` | Avanza una orden: iniciar y ejecutar (quien la tiene asignada o planifica), verificar (`mantenimiento.verificar`) o cancelar (`mantenimiento.planificar`). | `mantenimiento.planificar` `mantenimiento.ejecutar` `mantenimiento.verificar` | `AvancePeticion` | 200 `OrdenRespuesta` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/ordenes-trabajo/{orden_id}/evidencias` | Agrega una evidencia (foto antes/después o soporte). | `mantenimiento.planificar` `mantenimiento.ejecutar` | `EvidenciaDto` | 204  | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/obligaciones/sugeridas` | Catálogo sugerido de obligaciones periódicas (editable al activarlas: la periodicidad exacta depende de la norma local). | _No exige permisos: basta un rol en la copropiedad; módulo no contratado (`modulo_no_habilitado`)_ |  | 200 `[SugeridaRespuesta]` | 403 |
| GET | `/copropiedades/{copropiedad_id}/obligaciones` | Obligaciones periódicas, por próxima fecha. | `mantenimiento.planificar` `inventario.ver` |  | 200 `[ObligacionMantenimientoRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/obligaciones` | Crea una obligación (propia o del catálogo sugerido). | `mantenimiento.planificar` | `ObligacionPeticion` | 201 `ObligacionMantenimientoRespuesta` | 403, 409, 422 |
| PATCH | `/copropiedades/{copropiedad_id}/obligaciones/{obligacion_id}` | Edita una obligación (los campos omitidos no cambian). | `mantenimiento.planificar` | `ObligacionPeticion` | 200 `ObligacionMantenimientoRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/obligaciones/{obligacion_id}/cumplimientos` | Historial de cumplimientos, más recientes primero. | `mantenimiento.planificar` `inventario.ver` |  | 200 `[CumplimientoRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/obligaciones/{obligacion_id}/cumplimientos` | Registra que se cumplió; la próxima fecha se calcula desde la realización. | `mantenimiento.planificar` | `CumplimientoPeticion` | 201 `ObligacionMantenimientoRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/mantenimiento/alertas` | Alertas: obligaciones por vencer, garantías y órdenes vencidas. | `mantenimiento.planificar` `inventario.ver` |  | 200 `AlertasMantenimientoRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/mantenimiento/tablero` | Tablero: cumplimiento del plan preventivo, órdenes vencidas y costo por activo. | `mantenimiento.planificar` `inventario.ver` |  | 200 `TableroMantenimientoRespuesta` | 403 |

## importaciones

Importación masiva desde CSV/XLSX con vista previa

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| POST | `/copropiedades/{copropiedad_id}/importaciones/bienes-privados` | Sube y valida una planilla de bienes privados (CSV o XLSX). No aplica nada: devuelve la vista previa con los errores por fila. El tipo se detecta por el contenido. | `importaciones.ejecutar` | `application/octet-stream` | 201 `VistaPreviaRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/importaciones/bienes-privados/plantilla` | Plantilla CSV de bienes privados con ejemplos (copropietarios en filas seguidas). | _No exige permisos: basta un rol en la copropiedad_ |  | 200 `text/csv` | 403 |
| GET | `/copropiedades/{copropiedad_id}/importaciones/{importacion_id}` | Una importación con sus errores y su resumen. | `importaciones.ejecutar` |  | 200 `ImportacionRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/importaciones/{importacion_id}/confirmar` | Aplica una importación validada en una sola transacción: agrupaciones, bienes privados, propietarios (con su rol si tienen cuenta) y un cuadro **borrador** con los coeficientes. | `importaciones.ejecutar` |  | 200 `ImportacionRespuesta` | 403, 409 |

## asambleas

Asambleas y votaciones (Ley 675, arts. 37–49): convocatoria con padrón y constancias, mesa de registro asistido, poderes, morosos, quórum en vivo, preguntas (sí/no, selección, elección por cociente), voto del asistente por todos sus bienes, acta en PDF, reportes y papeleta virtual

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/copropiedades/{copropiedad_id}/asambleas` | Reuniones de la copropiedad, más recientes primero. | `asambleas.ver` |  | 200 `PaginaRespuesta_ReunionRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/asambleas` | Crea una reunión en borrador con su configuración (poderes, mora, base de cómputo). | `asambleas.convocar` | `ReunionPeticion` | 201 `ReunionRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}` | Reunión con su orden del día y anexos. | `asambleas.ver` |  | 200 `DetalleReunionRespuesta` | 403 |
| PATCH | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}` | Edita la reunión (la elección de la mora queda auditada). | `asambleas.convocar` | `EditarReunionPeticion` | 200 `ReunionRespuesta` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/puntos` | Agrega un punto al orden del día (en borrador). | `asambleas.convocar` | `PuntoPeticion` | 201 `PuntoRespuesta` | 403, 409 |
| PUT | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/puntos/{punto_id}` | Edita un punto (en borrador). | `asambleas.convocar` | `PuntoPeticion` | 200 `PuntoRespuesta` | 403 |
| DELETE | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/puntos/{punto_id}` | Elimina un punto (en borrador). | `asambleas.convocar` |  | 204  | 403 |
| PUT | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/puntos/{punto_id}/estado` | Avanza un punto del orden del día durante la reunión. | `asambleas.conducir` | `EstadoPuntoPeticion` | 200 `PuntoRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/documentos` | Anexa un documento a la convocatoria. | `asambleas.convocar` | `DocumentoPeticion` | 201 `DocumentoDto` | 403, 409 |
| DELETE | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/documentos/{archivo_id}` | Quita un anexo. | `asambleas.convocar` |  | 204  | 403 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/convocatoria` | Convoca: fija el padrón (cuadro de coeficientes vigente y representantes) y envía la convocatoria por WhatsApp y correo con constancia. Ordinarias de primera convocatoria: al menos 15 días calendario de antelación (art. 39). | `asambleas.convocar` | `ConvocarPeticion` | 200 `ConvocatoriaRespuesta` | 403, 409, 422 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/convocatoria/constancias` | Constancias de envío de la convocatoria (estado y acuse de cada mensaje). | `asambleas.convocar` |  | 200 `[ConstanciaRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/segunda-convocatoria` | Sin quórum en primera convocatoria: cancela la reunión y convoca la de segunda para el tercer día hábil siguiente a las 8:00 p. m. (art. 41). | `asambleas.convocar` | `ConvocarPeticion` | 201 `ConvocatoriaRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/transiciones` | Abre el registro, instala (exige quórum), suspende, reanuda, cierra (cierra las preguntas abiertas y envía el resumen por WhatsApp) o cancela. | `asambleas.conducir` `asambleas.convocar` | `TransicionPeticion` | 200 `ReunionRespuesta` | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/quorum` | Quórum actual por coeficiente y por unidades. | `asambleas.ver` |  | 200 `QuorumRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/quorum` | Toma una foto manual del quórum para el acta. | `asambleas.conducir` |  | 201 `QuorumRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/tablero` | Tablero en vivo (el mismo que emite el WebSocket). | `asambleas.ver` |  | 200 `TableroRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/en-vivo` | WebSocket del tablero en vivo: al conectar envía el tablero y luego uno nuevo cada vez que algo cambia (a lo sumo cada ~500 ms). El navegador envía el token como subprotocolo: `new WebSocket(url, ["dad.v1", token])`. | `asambleas.ver` |  |   | 101, 403 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/mesa` | Busca en el padrón por código del bien o documento del representante (sin tildes ni mayúsculas). Trae el representante, su teléfono del censo y sus otros bienes. | `asambleas.registrar` |  | 200 `[CoincidenciaRespuesta]` | 403 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/asistentes` | Asistentes con el estado de entrega de su confirmación por WhatsApp. | `asambleas.registrar` `asambleas.ver` |  | 200 `[AsistenteRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/asistentes` | Registra al asistente al firmar la planilla: cuenta para el quórum de inmediato; con un toque agrega sus otros bienes y los poderes. Le envía la plantilla de confirmación por WhatsApp y entrega el token de su papeleta virtual. | `asambleas.registrar` | `RegistroAsistentePeticion` | 201 `RegistroRespuesta` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/asistentes/{asistente_id}/poderes` | Agrega un poder a un asistente ya registrado. | `asambleas.registrar` | `PoderPeticion` | 201 `AsistenteRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/asistentes/{asistente_id}/salida` | Registra la salida con hora y motivo: sus bienes dejan el quórum y las preguntas que se abran después; sus votos se conservan. | `asambleas.registrar` | `SalidaPeticion` | 200 `AsistenteRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/asistentes/{asistente_id}/reingreso` | Registra el reingreso (nuevo periodo de presencia). | `asambleas.registrar` |  | 200 `AsistenteRespuesta` | 403, 409 |
| PUT | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/asistentes/{asistente_id}/rotulo` | Liga (o cambia) el rótulo preimpreso del asistente. | `asambleas.registrar` | `RotuloPeticion` | 200 `AsistenteRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/asistentes/{asistente_id}/confirmacion` | Reenvía la plantilla `confirmar_asistencia` (p. ej. con otro número si no llegó). | `asambleas.registrar` | `ConfirmacionPeticion` | 200 `AsistenteRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/poderes` | Poderes de la reunión. | `asambleas.registrar` |  | 200 `[PoderRespuesta]` | 403 |
| PUT | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/poderes/{poder_id}/estado` | Verifica, rechaza o revoca un poder (rechazado o revocado, el bien deja de estar representado por el apoderado). | `asambleas.registrar` | `RevisionPoderPeticion` | 200 `PoderRespuesta` | 403, 409 |
| PUT | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/morosos` | Fija el listado de morosos (después de convocar y antes de instalar). El efecto lo define la configuración de la reunión y queda auditado. | `asambleas.convocar` | `MorososPeticion` | 200 `MorososAsambleaRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/morosos/planilla` | Importa los morosos desde un CSV o XLSX (primera columna: código del bien). | `asambleas.convocar` | `application/octet-stream` | 200 `MorososAsambleaRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/preguntas` | Preguntas de la reunión, en orden. | `asambleas.ver` |  | 200 `[PreguntaRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/preguntas` | Crea una pregunta en borrador (antes o durante la reunión). | `asambleas.convocar` `asambleas.conducir` | `PreguntaPeticion` | 201 `PreguntaRespuesta` | 403, 422 |
| PUT | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/preguntas/{pregunta_id}` | Reemplaza la definición de una pregunta en borrador. | `asambleas.convocar` `asambleas.conducir` | `PreguntaPeticion` | 200 `PreguntaRespuesta` | 403, 409 |
| DELETE | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/preguntas/{pregunta_id}` | Elimina una pregunta en borrador. | `asambleas.convocar` `asambleas.conducir` |  | 204  | 403, 409 |
| PUT | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/preguntas/{pregunta_id}/estado` | Abre (habilita a los presentes y les envía la pregunta por WhatsApp), cierra (guarda el escrutinio) o anula la pregunta. | `asambleas.conducir` | `EstadoPreguntaPeticion` | 200 `PreguntaRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/preguntas/{pregunta_id}/votos` | Registra el voto del asistente por **todos** sus bienes habilitados. Exige `asambleas.votos.registrar` y la cabecera `Idempotency-Key` (una por lectura: la consola de la mesa la conserva si reintenta tras un corte). | `asambleas.votos.registrar` | `VotoPeticion` | 201 `VotoRespuesta` | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/preguntas/{pregunta_id}/resultados` | Resultados (en vivo si está abierta; los guardados si cerró). En reservadas se publican solo los totales. | `asambleas.ver` |  | 200 `ResultadosRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/preguntas/{pregunta_id}/detalle` | Detalle bien → opción. En reservadas exige `asambleas.votos.ver_detalle` y cada consulta queda auditada. | `asambleas.votos.ver_detalle` |  | 200 `[DetalleVotoRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/preguntas/{pregunta_id}/eleccion` | Aplica una elección cerrada de consejo, revisor fiscal o comité: crea los roles de los elegidos (origen `asamblea`) y termina los del periodo anterior. | `asambleas.conducir` | `EleccionPeticion` | 200 `[ElegidoRespuesta]` | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/acta` | Datos del acta para revisarla antes de generar el PDF (todo ya calculado). | `asambleas.ver` |  | 200 `DatosActa` | 403 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/acta` | Genera (o regenera mientras no se apruebe) el acta en PDF con Typst y la guarda como archivo con datos personales (`acta_archivo_id`). | `asambleas.conducir` |  | 200 `ReunionRespuesta` | 403, 409 |
| PUT | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/acta/estado` | Sube el escaneo firmado, aprueba o publica el acta. | `asambleas.conducir` | `PasoActaPeticion` | 200 `ReunionRespuesta` | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/actas` | Libro de actas: reuniones con el acta publicada. | `asambleas.ver` |  | 200 `[ReunionRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/asambleas/rotulos` | Hoja de rótulos genéricos numerados en Code 128 (PDF para imprimir). | `asambleas.registrar` | `RotulosPeticion` | 200 `application/pdf` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/reportes/asistencia` | Asistencia (un renglón por bien representado), en JSON o CSV. | `asambleas.ver` |  | 200 `[FilaAsistenciaRespuesta]` | 403 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/reportes/votos/{pregunta_id}` | Votos de una pregunta (en reservadas, la opción solo con permiso y auditada). | `asambleas.ver` `asambleas.votos.ver_detalle` |  | 200 `[FilaVotoRespuesta]` | 403 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/reportes/canales` | Participación por canal y tiempos de cada pregunta. | `asambleas.ver` |  | 200 `[CanalesRespuesta]` | 403 |
| GET | `/copropiedades/{copropiedad_id}/asambleas/{reunion_id}/reportes/costo-whatsapp` | Costo estimado de WhatsApp de la asamblea (para cobrarlo dentro de su valor). | `asambleas.convocar` |  | 200 `CostoRespuesta` | 403 |
| GET | 🔓 `/copropiedades/{copropiedad_id}/papeleta` | Papeleta: reunión, asistente con sus bienes y preguntas abiertas. |  |  | 200 `PapeletaRespuesta` | 429 |
| POST | 🔓 `/copropiedades/{copropiedad_id}/papeleta/ingreso` | Ingreso virtual (reuniones virtuales o mixtas): cuenta para el quórum desde ahora. |  |  | 200 `AsistenteRespuesta` | 409, 429 |
| POST | 🔓 `/copropiedades/{copropiedad_id}/papeleta/salida` | «Salir de la asamblea»: sus votos se conservan. |  |  | 200 `AsistenteRespuesta` | 409, 429 |
| POST | 🔓 `/copropiedades/{copropiedad_id}/papeleta/votos` | Vota por **todos** sus bienes habilitados (canal web). Exige `Idempotency-Key`. | _Papeleta inválida o el asistente no está presente (`no_inscrito`) o módulo no contratado (`modulo_no_habilitado`)_ | `VotoPapeletaPeticion` | 201 `VotoRespuesta` | 403, 409, 429 |

## acceso

Control de acceso (plan 05): visitas con aviso al primer respondiente y escalamiento (2 min a todos los residentes, 5 min llamada), pre-autorizaciones con QR firmado, correspondencia con avisos cada 24 h y entrega con firma o foto, minuta, bitácora, credenciales, autorizaciones, biometría con consentimiento y la API de los puentes (`/api/v1/puentes`)

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/puentes/sincronizacion` | Cambios desde el cursor del puente (cuenta como latido). | _Token de máquina inválido o puente retirado_ |  | 200 `SincronizacionRespuesta` | 403, 429 |
| POST | `/puentes/eventos` | Recibe un lote de eventos (máx. 500) y confirmaciones de borrado biométrico. |  | `ReportePeticion` | 200 `ReporteRespuesta` | 422, 429 |
| POST | `/puentes/latido` | Latido del puente (sin él por más de 5 min aparece en `/acceso/alertas`). |  | `LatidoPeticion` | 204  | 429 |
| GET | `/copropiedades/{copropiedad_id}/visitas` | Visitas, más recientes primero. La portería ve todas; un residente, las de su bien. | `acceso.registrar` `acceso.bitacora.ver` `acceso.visitantes.autorizar` |  | 200 `PaginaRespuesta_VisitaRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/visitas` | La portería anuncia una visita: aviso al primer respondiente (app o WhatsApp); a los 2 min a todos los residentes adultos y a los 5 min queda en `requiere_llamada`. | `acceso.registrar` | `AnunciarPeticion` | 201 `VisitaRespuesta` | 403, 422 |
| POST | `/copropiedades/{copropiedad_id}/visitas/preautorizaciones` | Un residente pre-autoriza una visita y recibe el QR temporal para compartirlo. | `acceso.visitantes.autorizar` | `PreautorizarPeticion` | 201 `VisitaConQrRespuesta` | 403, 422 |
| POST | `/copropiedades/{copropiedad_id}/visitas/ingreso-qr` | Ingreso con el QR temporal: verifica la firma de la PH, la ventana y los usos. | `acceso.registrar` | `IngresoQrPeticion` | 200 `VisitaRespuesta` | 403, 409, 422 |
| GET | `/copropiedades/{copropiedad_id}/visitas/{visita_id}` | Visita con su historial (avisos, decisiones, llamadas, ingreso y salida). | `acceso.registrar` `acceso.bitacora.ver` `acceso.visitantes.autorizar` |  | 200 `DetalleVisitaRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/visitas/{visita_id}/qr` | QR vigente de una visita pre-autorizada (se regenera igual: la firma es determinista). | `acceso.registrar` `acceso.bitacora.ver` `acceso.visitantes.autorizar` |  | 200 `QrRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/visitas/{visita_id}/respuesta` | Un residente autoriza o rechaza la visita desde la app o la web. | `acceso.visitantes.autorizar` | `ResponderPeticion` | 200 `DecisionRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/visitas/{visita_id}/contactos` | Nombres y teléfonos para llamar (solo mientras la visita está en `requiere_llamada`; cada consulta se audita). | `acceso.registrar` |  | 200 `[ContactoRespuesta]` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/visitas/{visita_id}/llamada` | La portería registra el resultado de la llamada. | `acceso.registrar` | `LlamadaPeticion` | 200 `VisitaRespuesta` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/visitas/{visita_id}/ingreso` | Ingreso de una visita autorizada. | `acceso.registrar` | `PuntoVisitaPeticion` | 200 `VisitaRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/visitas/{visita_id}/salida` | Salida de la visita (una pre-autorización con usos vuelve a quedar autorizada). | `acceso.registrar` | `PuntoVisitaPeticion` | 200 `VisitaRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/visitas/{visita_id}/cancelacion` | Cancela una visita que aún no ingresó. | `acceso.registrar` |  | 200 `VisitaRespuesta` | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/correspondencia` | Correspondencia, más reciente primero. Un residente ve la de su bien («mis paquetes»). | `acceso.registrar` `acceso.bitacora.ver` `acceso.correspondencia.entregar` |  | 200 `PaginaRespuesta_CorrespondenciaRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/correspondencia` | Registra la correspondencia y avisa al primer respondiente (o al terminar el horario de silencio). Cada 24 h sin recoger se avisa a todos los residentes, hasta el tope de días. | `acceso.registrar` | `RegistrarCorrespondenciaPeticion` | 201 `CorrespondenciaRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/correspondencia/personas-para-entrega` | Residentes adultos y propietarios del bien, para elegir a quien recoge. | `acceso.correspondencia.entregar` |  | 200 `[PersonaEntregaRespuesta]` | 403 |
| GET | `/copropiedades/{copropiedad_id}/correspondencia/{correspondencia_id}` | Correspondencia con su entrega (el documento del tercero solo lo ve la portería). | `acceso.registrar` `acceso.bitacora.ver` `acceso.correspondencia.entregar` |  | 200 `DetalleCorrespondenciaRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/correspondencia/{correspondencia_id}/entrega` | Entrega la correspondencia (se exige firma o foto) y confirma al primer respondiente. | `acceso.correspondencia.entregar` | `EntregarPeticion` | 200 `DetalleCorrespondenciaRespuesta` | 400, 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/correspondencia/{correspondencia_id}/devolucion` | Devuelve la correspondencia al remitente (deja de avisarse). | `acceso.correspondencia.entregar` | `DevolverPeticion` | 200 `CorrespondenciaRespuesta` | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/acceso/configuracion` | Configuración vigente (o la predeterminada). | `acceso.configurar` `acceso.registrar` |  | 200 `ConfiguracionAccesoDto` | 403 |
| PUT | `/copropiedades/{copropiedad_id}/acceso/configuracion` | Reemplaza la configuración (cada PH edita sus plazos de retención y aviso). | `acceso.configurar` | `ConfiguracionAccesoDto` | 200 `ConfiguracionAccesoDto` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/acceso/alertas` | Puentes sin latido, puertas forzadas (24 h) e intentos denegados repetidos (10 min). | `acceso.bitacora.ver` |  | 200 `[AlertaRespuesta]` | 403 |
| GET | `/copropiedades/{copropiedad_id}/acceso/costo-avisos` | Avisos de portería del periodo y su costo estimado de WhatsApp (el push es gratuito). | `acceso.configurar` |  | 200 `CostoAvisosRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/minuta` | Minuta de portería, más reciente primero. | `acceso.minuta.ver` |  | 200 `PaginaRespuesta_NovedadRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/minuta` | Registra una novedad (append-only) con sus adjuntos. | `acceso.minuta.registrar` | `NovedadPeticion` | 201 `NovedadRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/eventos-acceso` | Bitácora de eventos de acceso (portería y puentes), más recientes primero. | `acceso.bitacora.ver` |  | 200 `PaginaRespuesta_EventoAccesoRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/eventos-acceso` | Registra un evento manual en la bitácora (apertura del vigilante, novedad en un punto). | `acceso.registrar` | `EventoManualPeticion` | 201 `EventoAccesoRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/acceso/bienes` | Bienes privados activos por código, para anunciar visitas y recibir correspondencia. | `acceso.registrar` |  | 200 `PaginaRespuesta_BienPorteriaRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/acceso/personas` | Personas por nombre o documento, para emitir credenciales o registrar el consentimiento biométrico. Exige al menos 2 caracteres. | `acceso.credenciales.gestionar` `acceso.biometria.enrolar` |  | 200 `PaginaRespuesta_PersonaPorteriaRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/puntos-acceso` | Puntos de acceso de la copropiedad. | `acceso.registrar` `acceso.configurar` |  | 200 `[PuntoDto]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/puntos-acceso` | Crea un punto de acceso. | `acceso.configurar` | `PuntoDto` | 201 `PuntoDto` | 403, 409 |
| PUT | `/copropiedades/{copropiedad_id}/puntos-acceso/{punto_id}` | Edita un punto de acceso. | `acceso.configurar` | `PuntoDto` | 200 `PuntoDto` | 403 |
| GET | `/copropiedades/{copropiedad_id}/puentes` | Puentes de la copropiedad. | `acceso.configurar` |  | 200 `[PuenteRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/puentes` | Registra un puente (equipo nuevo o existente en la PH) y entrega su token. | `acceso.configurar` | `PuentePeticion` | 201 `PuenteRegistradoRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/puentes/{puente_id}/credencial` | Rota el token de máquina: el anterior deja de servir de inmediato. | `acceso.configurar` |  | 200 `TokenPuenteRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/puentes/{puente_id}/retiro` | Retira el puente (ya no sincroniza ni reporta). | `acceso.configurar` |  | 200 `PuenteRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/dispositivos-acceso` | Dispositivos (de un puente o todos). | `acceso.configurar` |  | 200 `[DispositivoDto]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/dispositivos-acceso` | Registra un dispositivo en un puente. | `acceso.configurar` | `DispositivoDto` | 201 `DispositivoDto` | 403, 422 |
| PUT | `/copropiedades/{copropiedad_id}/dispositivos-acceso/{dispositivo_id}` | Edita un dispositivo. | `acceso.configurar` | `DispositivoDto` | 200 `DispositivoDto` | 403 |
| GET | `/copropiedades/{copropiedad_id}/credenciales` | Credenciales de una persona o vehículo. | `acceso.credenciales.gestionar` |  | 200 `[CredencialRespuesta]` | 400, 403 |
| POST | `/copropiedades/{copropiedad_id}/credenciales` | Emite una credencial. Las biométricas exigen `acceso.biometria.enrolar` y consentimiento. | `acceso.credenciales.gestionar` | `EmitirPeticion` | 201 `EmitidaRespuesta` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/credenciales/verificacion` | Verifica lo leído en la portería digital y lo deja en la bitácora. | `acceso.registrar` | `VerificarPeticion` | 200 `VerificacionCredencialRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/credenciales/{credencial_id}/estado` | Suspende, reactiva, revoca o reporta perdida una credencial (el puente la retira en la siguiente sincronización). | `acceso.credenciales.gestionar` | `EstadoPeticion` | 200 `CredencialRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/credenciales/{credencial_id}/rotacion` | Rota un QR permanente o de la app (revoca el actual y emite otro). | `acceso.credenciales.gestionar` |  | 200 `EmitidaRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/autorizaciones-acceso` | Autorizaciones manuales de una persona o vehículo. | `acceso.credenciales.gestionar` |  | 200 `[AutorizacionAccesoRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/autorizaciones-acceso` | Crea una autorización manual (personal de servicio, proveedores frecuentes, horarios especiales). | `acceso.credenciales.gestionar` | `AutorizacionAccesoPeticion` | 201 `AutorizacionAccesoRespuesta` | 403, 422 |
| POST | `/copropiedades/{copropiedad_id}/autorizaciones-acceso/{autorizacion_id}/revocacion` | Revoca una autorización manual. | `acceso.credenciales.gestionar` |  | 204  | 403 |
| GET | `/copropiedades/{copropiedad_id}/personas/{persona_id}/consentimiento-biometrico` | Consentimiento biométrico vigente (o `null`). | `acceso.biometria.enrolar` |  | 200  | 403 |
| POST | `/copropiedades/{copropiedad_id}/personas/{persona_id}/consentimiento-biometrico` | Registra el consentimiento biométrico explícito (Ley 1581 de 2012: facultativo, con método alterno siempre disponible). | `acceso.biometria.enrolar` | `ConsentimientoPeticion` | 201 `ConsentimientoRespuesta` | 403, 409 |
| DELETE | `/copropiedades/{copropiedad_id}/personas/{persona_id}/consentimiento-biometrico` | Revoca el consentimiento (la persona misma o quien enrola): las credenciales biométricas se revocan y el puente borra las plantillas del equipo. | `acceso.biometria.enrolar` |  | 200 `RevocacionRespuesta` | 403 |

## proteccion_datos

Protección de datos personales (Ley 1581 de 2012): política de tratamiento versionada, autorizaciones por finalidad, consultas y reclamos del titular con plazos en días hábiles y prórroga, «mis datos»

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/copropiedades/{copropiedad_id}/proteccion-datos/politica` | Política vigente (la lee cualquier cuenta de la PH antes de autorizar); `null` si no hay. | _No exige permisos: basta un rol en la copropiedad_ |  | 200  | 403 |
| GET | `/copropiedades/{copropiedad_id}/proteccion-datos/politicas` | Todas las versiones publicadas, la más reciente primero. | `proteccion_datos.gestionar` |  | 200 `[PoliticaRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/proteccion-datos/politicas` | Publica una versión nueva de la política (las autorizaciones anteriores conservan la suya). | `proteccion_datos.gestionar` | `PoliticaPeticion` | 201 `PoliticaRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/proteccion-datos/mis-datos` | «Mis datos»: la ficha de la cuenta en la PH, sus autorizaciones y solicitudes. Se audita. | `proteccion_datos.gestionar` |  | 200 `DatosTitularRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/personas/{persona_id}/datos-tratados` | Lo que la PH trata de una persona (para responder una consulta). Se audita. | `proteccion_datos.gestionar` |  | 200 `DatosTitularRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/personas/{persona_id}/autorizaciones-tratamiento` | Autorizaciones (vigentes y revocadas) de la persona. | `proteccion_datos.gestionar` |  | 200 `[AutorizacionTratamientoRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/personas/{persona_id}/autorizaciones-tratamiento` | Registra la autorización del titular bajo la política vigente. | `proteccion_datos.gestionar` | `OtorgarPeticion` | 201 `FinalidadesRespuesta` | 403, 422 |
| POST | `/copropiedades/{copropiedad_id}/personas/{persona_id}/autorizaciones-tratamiento/revocacion` | Revoca la autorización para esas finalidades (art. 8, lit. e). | `proteccion_datos.gestionar` | `RevocarPeticion` | 200 `RevocadasRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/proteccion-datos/solicitudes` | Solicitudes, más recientes primero. El titular ve las suyas; la administración, todas. | `proteccion_datos.gestionar` |  | 200 `PaginaRespuesta_SolicitudTitularRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/proteccion-datos/solicitudes` | Radica una consulta o un reclamo; el vencimiento se calcula en días hábiles de Colombia. | `proteccion_datos.gestionar` | `RadicarSolicitudPeticion` | 201 `SolicitudTitularRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/proteccion-datos/solicitudes/{solicitud_id}/tramite` | La administración empieza a atender la solicitud. | `proteccion_datos.gestionar` |  | 200 `SolicitudTitularRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/proteccion-datos/solicitudes/{solicitud_id}/prorroga` | Prórroga única antes del vencimiento (5 días hábiles las consultas, 8 los reclamos). | `proteccion_datos.gestionar` | `ProrrogaPeticion` | 200 `SolicitudTitularRespuesta` | 403, 422 |
| POST | `/copropiedades/{copropiedad_id}/proteccion-datos/solicitudes/{solicitud_id}/respuesta` | Responde la solicitud (texto, archivo o ambos). | `proteccion_datos.gestionar` | `RespuestaSolicitudPeticion` | 200 `SolicitudTitularRespuesta` | 403, 409, 422 |

## auditoria

Consulta de la auditoría de la copropiedad (solo lectura; nunca trae datos personales)

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/entidades/{entidad_id}/auditoria` | Eventos de la auditoría, más recientes primero. | `auditoria.ver` |  | 200 `PaginaRespuesta_EventoAuditadoRespuesta` | 403, 422 |

## publicidad

Publicidad de la plataforma (plan 08): portal del anunciante en `/anunciantes`, moderación, tarifas, política y liquidación en `/plataforma/publicidad`, y en la copropiedad las piezas, eventos, cupones y reportes solo para residentes que autorizaron la publicidad. Las copropiedades no venden ni configuran pauta

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/plataforma/publicidad/anunciantes/{anunciante_id}/campanas` | Campañas del anunciante. | `plataforma.publicidad.gestionar` |  | 200 `PaginaRespuesta_CampanaRespuesta` | 403 |
| POST | `/plataforma/publicidad/anunciantes/{anunciante_id}/campanas` | Crea una campaña en borrador. | `plataforma.publicidad.gestionar` | `CampanaPeticion` | 201 `CampanaRespuesta` | 403, 422 |
| GET | `/plataforma/publicidad/anunciantes/{anunciante_id}/campanas/{campana_id}` | Campaña con sus creativos. | `plataforma.publicidad.gestionar` |  | 200 `CampanaDetalleRespuesta` | 403 |
| PUT | `/plataforma/publicidad/anunciantes/{anunciante_id}/campanas/{campana_id}` | Edita la campaña (una aprobada o pausada vuelve a borrador y a moderación). | `plataforma.publicidad.gestionar` | `CampanaPeticion` | 200 `CampanaRespuesta` | 403, 422 |
| POST | `/plataforma/publicidad/anunciantes/{anunciante_id}/campanas/{campana_id}/estado` | Envía a revisión, activa, pausa o finaliza la campaña. | `plataforma.publicidad.gestionar` | `AccionCampanaPeticion` | 200 `CampanaRespuesta` | 403, 409, 422 |
| POST | `/plataforma/publicidad/anunciantes/{anunciante_id}/campanas/{campana_id}/creativos` | Agrega un creativo (queda pendiente de moderación). | `plataforma.publicidad.gestionar` | `CreativoPeticion` | 201 `CreativoRespuesta` | 403, 409, 422 |
| PUT | `/plataforma/publicidad/anunciantes/{anunciante_id}/campanas/{campana_id}/creativos/{creativo_id}` | Edita un creativo (si cambia, vuelve a moderación). | `plataforma.publicidad.gestionar` | `CreativoPeticion` | 200 `CreativoRespuesta` | 403, 409 |
| GET | `/plataforma/publicidad/anunciantes/{anunciante_id}/campanas/{campana_id}/metricas` | Métricas agregadas de la campaña. | `plataforma.publicidad.gestionar` |  | 200 `MetricasRespuesta` | 403 |
| GET | `/plataforma/publicidad/anunciantes/{anunciante_id}/leads` | Contactos que los residentes autorizaron compartirle al comercio (nombre y celular, para esa solicitud). | `plataforma.publicidad.gestionar` |  | 200 `PaginaRespuesta_LeadRespuesta` | 403 |
| GET | `/plataforma/publicidad/anunciantes/{anunciante_id}/enlace-cupones` | El enlace de redención del comercio, sin cambiarlo. | `plataforma.publicidad.gestionar` |  | 200 `EnlaceTokenRespuesta` | 403 |
| POST | `/plataforma/publicidad/anunciantes/{anunciante_id}/enlace-cupones` | Genera (o reemplaza, el anterior deja de servir) el enlace con que el comercio redime cupones. | `plataforma.publicidad.gestionar` |  | 201 `EnlaceTokenRespuesta` | 403, 422 |
| DELETE | `/plataforma/publicidad/anunciantes/{anunciante_id}/enlace-cupones` | Revoca el enlace del comercio. | `plataforma.publicidad.gestionar` |  | 204  | 403 |
| GET | `/plataforma/publicidad/exclusiones` | Entidades excluidas de la publicidad (no reciben piezas ni destacadas). | `plataforma.publicidad.gestionar` |  | 200 `[ExclusionRespuesta]` | 403 |
| POST | `/plataforma/publicidad/exclusiones` | Excluye una entidad de la publicidad. Solo la plataforma lo decide: la entidad no controla la publicidad. Su directorio sigue funcionando. | `plataforma.publicidad.gestionar` | `ExclusionPeticion` | 201 `ExclusionRespuesta` | 403, 422 |
| DELETE | `/plataforma/publicidad/exclusiones/{entidad_id}` | Vuelve a incluir una entidad en la publicidad. | `plataforma.publicidad.gestionar` |  | 204  | 403 |
| GET | `/plataforma/publicidad/moderacion/creativos` | Creativos pendientes de moderación. | `plataforma.publicidad.moderar` |  | 200 `PaginaRespuesta_CreativoRespuesta` | 403 |
| POST | `/plataforma/publicidad/moderacion/creativos/{creativo_id}` | Aprueba o rechaza un creativo. | `plataforma.publicidad.moderar` | `DecisionPeticion` | 200 `CreativoRespuesta` | 403, 409 |
| GET | `/plataforma/publicidad/moderacion/campanas` | Campañas en revisión (con el conflicto de interés marcado). | `plataforma.publicidad.moderar` |  | 200 `PaginaRespuesta_CampanaRespuesta` | 403 |
| POST | `/plataforma/publicidad/moderacion/campanas/{campana_id}` | Aprueba (fija la tarifa vigente) o rechaza una campaña. | `plataforma.publicidad.moderar` | `DecisionPeticion` | 200 `CampanaRespuesta` | 403, 422 |
| GET | `/plataforma/publicidad/moderacion/reportes` | Creativos con reportes sin atender, de todas las copropiedades. | `plataforma.publicidad.moderar` |  | 200 `PaginaRespuesta_CreativoReportadoRespuesta` | 403 |
| POST | `/plataforma/publicidad/moderacion/reportes/{creativo_id}` | Atiende los reportes de un creativo: lo retira o lo mantiene. | `plataforma.publicidad.moderar` | `AtenderReportesPeticion` | 200 `ReportesAtendidosRespuesta` | 403 |
| GET | `/plataforma/publicidad/tarifas` | Todas las tarifas. | `plataforma.publicidad.liquidar` |  | 200 `[TarifaPublicidadDto]` | 403 |
| POST | `/plataforma/publicidad/tarifas` | Fija la tarifa de un modelo desde una fecha (hoy o futura). | `plataforma.publicidad.liquidar` | `TarifaPublicidadDto` | 201 `TarifaPublicidadDto` | 403, 409 |
| POST | `/plataforma/publicidad/politica` | Publica una versión nueva de la política de publicidad (rige desde ya). | `plataforma.publicidad.liquidar` | `PoliticaPublicidadPeticion` | 201 `PoliticaPublicidadRespuesta` | 403 |
| GET | `/plataforma/publicidad/liquidaciones` | Liquidaciones de un mes. | `plataforma.publicidad.liquidar` |  | 200 `[LiquidacionRespuesta]` | 403 |
| POST | `/plataforma/publicidad/liquidaciones` | Calcula o recalcula las liquidaciones de un mes ya cerrado. | `plataforma.publicidad.liquidar` | `MesQuery` | 200 `[LiquidacionRespuesta]` | 403, 422 |
| POST | `/plataforma/publicidad/liquidaciones/{anunciante_id}/estado` | Marca la liquidación del anunciante facturada o pagada. | `plataforma.publicidad.liquidar` | `EstadoLiquidacionPeticion` | 200 `LiquidacionRespuesta` | 403 |
| GET | `/plataforma/publicidad/pantallas/{copropiedad_id}` | El token vigente de la pantalla del lobby, sin cambiarlo. | `plataforma.modulos.gestionar` |  | 200 `EnlaceTokenRespuesta` | 403 |
| POST | `/plataforma/publicidad/pantallas/{copropiedad_id}` | Genera el token de la pantalla del lobby de una copropiedad (reemplaza el anterior). | `plataforma.modulos.gestionar` |  | 201 `PantallaRespuesta` | 403 |
| POST | 🔓 `/publicidad/cupones/redimir` | Redime un cupón en el mostrador del comercio, con el token de **su enlace** en `X-Comercio` (sin sesión: los comercios no tienen cuenta en la plataforma). |  | `RedimirPeticion` | 200 `RedencionRespuesta` | 422, 429 |
| GET | 🔓 `/copropiedades/{copropiedad_id}/publicidad/pantalla` | Cartelera de la pantalla del lobby (sin sesión: el token de la pantalla en `X-Pantalla`). |  |  | 200 `[PiezaRespuesta]` | 429 |
| GET | `/entidades/{entidad_id}/publicidad/autorizacion` | ¿La cuenta autorizó recibir publicidad de la plataforma? Con la política vigente. | _No exige permisos: basta un rol en la copropiedad (sin persona en la copropiedad, 422)_ |  | 200 `AutorizacionPublicidadRespuesta` | 403, 422 |
| POST | `/entidades/{entidad_id}/publicidad/autorizacion` | El titular autoriza recibir publicidad de la plataforma (solo él; la PH no puede). | _No exige permisos: solo el titular autoriza (sin persona en la copropiedad, 422)_ | `AutorizarPublicidadPeticion` | 204  | 403, 422 |
| DELETE | `/entidades/{entidad_id}/publicidad/autorizacion` | Retira la autorización de publicidad. | _No exige permisos: solo el titular retira su autorización_ |  | 204  | 403, 422 |
| GET | `/entidades/{entidad_id}/publicidad/piezas` | Piezas para el residente (vacío si no autorizó la publicidad o la PH no tiene el módulo). | `directorio.ver` |  | 200 `[PiezaRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/publicidad/eventos` | Registra impresiones y clics de las piezas (solo de quien autorizó; hasta 100 por contador). | `directorio.ver` | `EventosPeticion` | 200 `EventosRespuesta` | 403, 422 |
| POST | `/entidades/{entidad_id}/publicidad/creativos/{creativo_id}/cupon` | El residente obtiene el código de una oferta para presentarlo en el comercio. | `directorio.ver` |  | 200 `CuponRespuesta` | 403 |
| POST | `/entidades/{entidad_id}/publicidad/creativos/{creativo_id}/reporte` | Reporta un anuncio para la moderación de la plataforma. | `directorio.ver` | `ReporteAnuncioPeticion` | 200 `ReporteAnuncioRespuesta` | 403, 422 |

## plataforma

Operación de la plataforma (exige segundo factor)

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/plataforma/copropiedades` | Todas las copropiedades de la plataforma. | `plataforma.copropiedades.ver` |  | 200 `[CopropiedadRespuesta]` | 403 |
| POST | `/plataforma/copropiedades` | Da de alta una copropiedad (en configuración) y le asigna su primer administrador. | `plataforma.copropiedades.crear` | `NuevaCopropiedadPeticion` | 201 `CopropiedadRespuesta` | 403, 409, 422 |
| POST | `/plataforma/copropiedades/{copropiedad_id}/estado` | Activa, suspende, reactiva o retira una copropiedad (alias de `POST /plataforma/entidades/{entidad_id}/estado`). | `plataforma.copropiedades.crear` | `EstadoCopropiedadPeticion` | 200 `EstadoCopropiedadRespuesta` | 403, 409 |
| POST | `/plataforma/copropiedades/{copropiedad_id}/modulos` | Habilita un módulo en una copropiedad (venta por módulo). Los endpoints del módulo dejan de responder `modulo_no_habilitado` desde `desde`. Si la entidad tiene suscripción y hay un producto con tarifa para el módulo, agrega su línea de cobro. | `plataforma.modulos.gestionar` | `HabilitarModuloPeticion` | 201 `PeriodoModuloRespuesta` | 403, 422 |
| DELETE | `/plataforma/copropiedades/{copropiedad_id}/modulos/{modulo}` | Termina un módulo en una copropiedad desde ahora; el historial de periodos se conserva. | `plataforma.modulos.gestionar` |  | 200 `ModuloTerminadoRespuesta` | 403, 422 |
| GET | `/plataforma/roles` | Roles de plataforma (plantillas y propios). | `plataforma.roles.asignar` `plataforma.roles.gestionar` |  | 200 `[RolRespuesta]` | 403 |
| POST | `/plataforma/roles` | Crea un rol de plataforma propio (solo permisos de plataforma). | `plataforma.roles.gestionar` | `NuevoRolPeticion` | 201 `RolRespuesta` | 403, 409 |
| PUT | `/plataforma/roles/{rol_id}/permisos` | Reemplaza los permisos de un rol de plataforma propio. | `plataforma.roles.gestionar` | `PermisosPeticion` | 200 `RolRespuesta` | 403 |
| GET | `/plataforma/asignaciones` | Historial de asignaciones de roles de plataforma, las más recientes primero. | `plataforma.roles.asignar` `plataforma.roles.gestionar` |  | 200 `[AsignacionPlataformaDetalleRespuesta]` | 403 |
| POST | `/plataforma/asignaciones` | Asigna un rol de plataforma (soporte, comercial, superusuario…). | `plataforma.roles.asignar` | `AsignacionPlataformaPeticion` | 201 `AsignacionPlataformaRespuesta` | 403 |
| DELETE | `/plataforma/asignaciones/{asignacion_id}` | Termina una asignación de plataforma (nunca el último superusuario). | `plataforma.roles.asignar` |  | 204  | 403, 409 |
| POST | `/plataforma/cuentas/{cuenta_id}/bloquear` | Bloquea una cuenta y cierra sus sesiones. | `plataforma.cuentas.bloquear` |  | 204  | 403 |
| GET | `/plataforma/cuentas` | Busca una cuenta por usuario o correo exactos: estado, contraseña pendiente y segundo factor. | `plataforma.cuentas.ver` |  | 200 `CuentaPlataformaRespuesta` | 403 |
| POST | `/plataforma/cuentas` | Crea un usuario de la plataforma (o reconoce la cuenta de ese correo) y, si se pide, le asigna un rol de plataforma. Recibe por correo el código para definir su contraseña. | `plataforma.cuentas.crear` `plataforma.roles.asignar` | `NuevoUsuarioPlataformaPeticion` | 201 `UsuarioPlataformaRespuesta` | 403, 409, 422 |
| POST | `/plataforma/cuentas/{cuenta_id}/desbloquear` | Desbloquea una cuenta bloqueada por la plataforma (y su bloqueo temporal por intentos). | `plataforma.cuentas.bloquear` | `MotivoPeticion` | 204  | 403, 422 |
| GET | `/plataforma/entidades` | PH, empresas y la plataforma en una sola tabla, con tipo, estado y módulos; los filtros se aplican en todas las páginas. | `plataforma.copropiedades.ver` |  | 200 `PaginaRespuesta_EntidadPlataformaRespuesta` | 403 |
| POST | `/plataforma/entidades` | Da de alta una entidad sin vertical (cualquier empresa de la contabilidad base); sus módulos se habilitan después, uno a uno. | `plataforma.entidades.crear` | `NuevaEntidadPeticion` | 201 `EntidadRespuesta` | 403, 409, 422 |
| GET | `/plataforma/entidades/{entidad_id}` | Una entidad por su id, con tipo, estado y módulos (la ficha del cliente en la consola). | `plataforma.copropiedades.ver` |  | 200 `EntidadPlataformaRespuesta` | 403 |
| POST | `/plataforma/entidades/{entidad_id}/estado` | Activa, suspende, reactiva o retira una entidad (PH o empresa). | `plataforma.copropiedades.crear` `plataforma.entidades.crear` | `EstadoCopropiedadPeticion` | 200 `EstadoCopropiedadRespuesta` | 403, 409 |
| POST | `/plataforma/entidades/{entidad_id}/soporte` | Abre un acceso de soporte de solo lectura a la entidad por 2 horas. Después la cuenta entra por las rutas normales de la entidad: ve lo que se lee (nunca datos sensibles, votos reservados ni convivencia) y toda escritura responde 403. | `plataforma.soporte.ver` | `SoportePeticion` | 201 `SoporteRespuesta` | 403, 422 |
| GET | `/plataforma/entidades/{entidad_id}/roles` | Roles de la entidad (plantillas y propios). | `plataforma.entidades.usuarios` |  | 200 `[RolRespuesta]` | 403 |
| GET | `/plataforma/entidades/{entidad_id}/asignaciones` | Historial de asignaciones de la entidad. | `plataforma.entidades.usuarios` |  | 200 `[AsignacionRespuesta]` | 403 |
| POST | `/plataforma/entidades/{entidad_id}/asignaciones` | Asigna un rol de la entidad a una cuenta existente. | `plataforma.entidades.usuarios` | `AsignacionPeticion` | 201 `AsignacionRespuesta` | 403 |
| DELETE | `/plataforma/entidades/{entidad_id}/asignaciones/{asignacion_id}` | Termina una asignación (la entidad no se queda sin administrador). | `plataforma.entidades.usuarios` |  | 204  | 403, 409 |
| GET | `/plataforma/entidades/{entidad_id}/usuarios` | Usuarios de la entidad con sus roles. | `plataforma.entidades.usuarios` |  | 200 `PaginaRespuesta_UsuarioRespuesta` | 403 |
| POST | `/plataforma/entidades/{entidad_id}/usuarios` | Crea un usuario en la entidad con su rol (o asigna el rol a la cuenta de ese correo). | `plataforma.entidades.usuarios` | `NuevoUsuarioPeticion` | 201 `UsuarioCreadoRespuesta` | 403, 409, 422 |
| POST | `/plataforma/entidades/{entidad_id}/usuarios/{cuenta_id}/codigo-contrasena` | Reenvía el código para definir la contraseña a un usuario de la entidad. | `plataforma.entidades.usuarios` |  | 204  | 403, 422 |
| GET | `/plataforma/reclamaciones` | Reclamaciones de organizaciones pendientes de revisión. | `plataforma.organizaciones.revisar` |  | 200 `[ReclamacionRespuesta]` | 403 |
| POST | `/plataforma/reclamaciones/{reclamacion_id}/resolver` | Aprueba (la cuenta queda como dueña de la ficha) o rechaza una reclamación. | `plataforma.organizaciones.revisar` | `ResolucionPeticion` | 204  | 403, 409 |
| GET | `/plataforma/whatsapp/lineas` | Líneas de la plataforma (cualquiera atiende cualquier copropiedad). | `plataforma.whatsapp.gestionar` |  | 200 `[LineaRespuesta]` | 403 |
| POST | `/plataforma/whatsapp/lineas` | Registra una línea. | `plataforma.whatsapp.gestionar` | `LineaPeticion` | 201 `LineaRespuesta` | 403, 409 |
| PATCH | `/plataforma/whatsapp/lineas/{linea_id}` | Cambia el estado, el ritmo o la franquicia de una línea. | `plataforma.whatsapp.gestionar` | `ActualizarLineaPeticion` | 204  | 403 |
| GET | `/plataforma/whatsapp/tarifas` | Tarifas registradas (la vigente es la más reciente por país, categoría y moneda). | `plataforma.whatsapp.gestionar` |  | 200 `[TarifaDto]` | 403 |
| POST | `/plataforma/whatsapp/tarifas` | Registra la tarifa vigente desde una fecha (Meta publica tablas nuevas cada trimestre). | `plataforma.whatsapp.gestionar` | `TarifaDto` | 201 `TarifaDto` | 403, 422 |
| PUT | `/plataforma/entidad-plataforma` | Designa la entidad que opera la plataforma (la empresa de GISLA SAS). Quien tiene roles de plataforma la recibe como `plataforma` al iniciar sesión. Solo hay una: designar otra le quita la marca a la anterior. | `plataforma.entidad_propia.designar` | `EntidadPlataformaPeticion` | 204  | 403, 409 |
| GET | `/plataforma/facturacion/consumo` | Documentos electrónicos transmitidos (validados, enviados o en contingencia) por entidad y tipo en el periodo, con `desde` y `hasta` **incluidos**: la base para cobrar el módulo de facturación (plan 19). Solo conteos. | `plataforma.facturacion.ver` |  | 200 `[ConsumoDocumentosRespuesta]` | 403, 422 |
| PUT | `/plataforma/puc` | Carga (inserta o actualiza) cuentas del catálogo PUC. | `plataforma.normativa.gestionar` | `CargaPucPeticion` | 200 `CargaPucRespuesta` | 403, 422 |
| GET | `/plataforma/retenciones` | Catálogo de retenciones (todas las versiones) y valores de la UVT. | `plataforma.normativa.gestionar` |  | 200 `CatalogoRetencionesRespuesta` | 403 |
| POST | `/plataforma/retenciones` | Agrega una versión de un concepto de retención (la anterior del mismo código queda vigente hasta el día antes de `vigente_desde`). | `plataforma.normativa.gestionar` | `NuevoConceptoRetencionPeticion` | 201 `ConceptoRetencionRespuesta` | 403, 409, 422 |
| PUT | `/plataforma/uvt/{anio}` | Crea o corrige el valor de la UVT del año. | `plataforma.normativa.gestionar` | `UvtPeticion` | 200 `UvtRespuesta` | 403, 422 |
| GET | `/plataforma/plantillas-contables` | Plantillas estándar de la plataforma (todas sus versiones y estados). | `plataforma.plantillas_contables.publicar` |  | 200 `[PlantillaResumenRespuesta]` | 403 |
| POST | `/plataforma/plantillas-contables` | Crea una plantilla estándar en borrador (p. ej. la de propiedad horizontal). | `plataforma.plantillas_contables.publicar` | `NuevaPlantillaPeticion` | 201 `PlantillaRespuesta` | 403, 422 |
| GET | `/plataforma/plantillas-contables/{plantilla_id}` | Plantilla con su contenido. | `plataforma.plantillas_contables.publicar` |  | 200 `PlantillaRespuesta` | 403 |
| PUT | `/plataforma/plantillas-contables/{plantilla_id}/contenido` | Reemplaza el contenido de una plantilla en borrador. | `plataforma.plantillas_contables.publicar` | `ContenidoPlantillaPeticion` | 204  | 403, 409, 422 |
| PUT | `/plataforma/plantillas-contables/{plantilla_id}/estado` | Publica o retira una plantilla estándar. | `plataforma.plantillas_contables.publicar` | `EstadoPlantillaPeticion` | 204  | 403, 409 |
| GET | `/plataforma/tributario/catalogo` | Catálogo tributario completo (todas las versiones). | `plataforma.normativa.gestionar` |  | 200 `CatalogoTributarioRespuesta` | 403 |
| POST | `/plataforma/tributario/tarifas-iva` | Agrega una versión de una tarifa de IVA (la anterior del mismo código queda vigente hasta el día antes). | `plataforma.normativa.gestionar` | `NuevaTarifaIvaPeticion` | 201 `TarifaIvaRespuesta` | 403, 409, 422 |
| POST | `/plataforma/tributario/tarifas-ica` | Agrega una versión de una tarifa de ICA por municipio y actividad. | `plataforma.normativa.gestionar` | `NuevaTarifaIcaPeticion` | 201 `TarifaIcaRespuesta` | 403, 409, 422 |
| POST | `/plataforma/tributario/formatos-exogena` | Agrega un formato de exógena (versión siguiente del año y código) con su mapeo de cuentas. | `plataforma.normativa.gestionar` | `NuevoFormatoExogenaPeticion` | 201 `FormatoExogenaRespuesta` | 403, 422 |
| GET | `/plataforma/tasas-referencia` | Tasas de referencia (interés bancario corriente) por vigencia. | `plataforma.normativa.gestionar` |  | 200 `[TasaReferenciaRespuesta]` | 403 |
| POST | `/plataforma/tasas-referencia` | Registra el interés bancario corriente certificado de un periodo. | `plataforma.normativa.gestionar` | `NuevaTasaPeticion` | 201 `TasaReferenciaRespuesta` | 403, 409, 422 |
| GET | `/plataforma/cobros/productos` | Catálogo comercial con las tarifas de cada producto. | `plataforma.comercial.gestionar` `plataforma.cobros.aprobar` |  | 200 `[ProductoRespuesta]` | 403 |
| POST | `/plataforma/cobros/productos` | Crea un producto del catálogo comercial. | `plataforma.comercial.gestionar` | `ProductoPeticion` | 201 `ProductoRespuesta` | 403, 409, 422 |
| PUT | `/plataforma/cobros/productos/{producto_id}` | Edita un producto (el código no cambia). | `plataforma.comercial.gestionar` | `ProductoPeticion` | 200 `ProductoRespuesta` | 403 |
| POST | `/plataforma/cobros/productos/{producto_id}/tarifas` | Fija la tarifa del producto desde una fecha (versionada: nunca se edita). | `plataforma.comercial.gestionar` | `TarifaCobroDto` | 201 `TarifaCobroDto` | 403, 409, 422 |
| GET | `/plataforma/cobros/suscripciones` | Suscripciones vigentes. | `plataforma.comercial.gestionar` `plataforma.cobros.aprobar` |  | 200 `[SuscripcionRespuesta]` | 403 |
| POST | `/plataforma/cobros/suscripciones` | Crea la suscripción de una entidad (una vigente por entidad). | `plataforma.comercial.gestionar` | `SuscripcionPeticion` | 201 `SuscripcionRespuesta` | 403, 409, 422 |
| GET | `/plataforma/cobros/suscripciones/{suscripcion_id}` | Suscripción con sus líneas y los módulos habilitados sin cobro. | `plataforma.comercial.gestionar` `plataforma.cobros.aprobar` |  | 200 `DetalleSuscripcionRespuesta` | 403 |
| PATCH | `/plataforma/cobros/suscripciones/{suscripcion_id}` | Cambia el correo o el plazo de pago, o termina la suscripción. | `plataforma.comercial.gestionar` | `CambiosSuscripcionPeticion` | 200 `SuscripcionRespuesta` | 403, 422 |
| POST | `/plataforma/cobros/suscripciones/{suscripcion_id}/lineas` | Agrega un producto a la suscripción. | `plataforma.comercial.gestionar` | `LineaSuscripcionPeticion` | 201 `LineaSuscripcionRespuesta` | 403, 422 |
| POST | `/plataforma/cobros/lineas/{linea_id}/fin` | Termina una línea (deja de cobrarse desde `hasta`). | `plataforma.comercial.gestionar` | `FinLineaPeticion` | 204  | 403, 422 |
| GET | `/plataforma/cobros/cortes` | Cortes (los más recientes primero), con filtro por estado o suscripción. | `plataforma.cobros.aprobar` `plataforma.cobros.facturar` |  | 200 `PaginaRespuesta_CorteRespuesta` | 403 |
| POST | `/plataforma/cobros/cortes` | Calcula (o recalcula, si sigue en borrador) el corte de un periodo. Si cobra lo mismo que el anterior queda aprobado solo. | `plataforma.cobros.aprobar` | `CalcularPeticion` | 200 `CorteRespuesta` | 403, 422 |
| GET | `/plataforma/cobros/cortes/{corte_id}` | Un corte. | `plataforma.cobros.aprobar` `plataforma.cobros.facturar` |  | 200 `CorteRespuesta` | 403 |
| POST | `/plataforma/cobros/cortes/{corte_id}/aprobar` | Aprueba un corte en borrador (queda «por facturar»). | `plataforma.cobros.aprobar` |  | 200 `CorteRespuesta` | 403, 409 |
| POST | `/plataforma/cobros/cortes/{corte_id}/descartar` | Descarta un corte en borrador o aprobado (con motivo). | `plataforma.cobros.aprobar` | `MotivoCortePeticion` | 200 `CorteRespuesta` | 403, 409 |
| POST | `/plataforma/cobros/cortes/{corte_id}/facturar` | Factura un corte aprobado en la entidad de la plataforma (factura de venta emitida; el documento electrónico se transmite desde su facturación) y avisa al correo de facturación. | `plataforma.cobros.facturar` |  | 200 `CorteRespuesta` | 403, 409, 422 |
| POST | `/plataforma/cobros/cortes/{corte_id}/factura-externa` | Marca un corte aprobado como facturado por fuera del sistema (mientras la plataforma no factura en él). | `plataforma.cobros.facturar` | `FacturaExternaPeticion` | 200 `CorteRespuesta` | 403, 409 |
| POST | `/plataforma/cobros/cortes/{corte_id}/pago` | Registra el pago de un corte facturado; si la entidad estaba suspendida por mora y no le queda otro vencido, se reactiva. | `plataforma.cobros.facturar` | `PagoCortePeticion` | 200 `CorteRespuesta` | 403, 409 |
| POST | `/plataforma/cobros/liquidaciones/{liquidacion_id}/corte` | Crea el corte (en borrador) de una liquidación de publicidad de un anunciante. | `plataforma.cobros.aprobar` |  | 200 `CorteRespuesta` | 403, 422 |
| GET | `/plataforma/nomina/catalogo` | Catálogo laboral completo de la plataforma (todas las versiones y administradoras). | `plataforma.normativa.gestionar` |  | 200 `CatalogoLaboralRespuesta` | 403 |
| POST | `/plataforma/nomina/parametros` | Agrega una versión de un parámetro laboral. El catálogo es de solo inserción: un error se corrige con otra versión. | `plataforma.normativa.gestionar` | `NuevoParametroLaboralPeticion` | 201 `ParametroLaboralRespuesta` | 403, 409, 422 |
| POST | `/plataforma/nomina/administradoras` | Agrega una administradora de la seguridad social al catálogo. | `plataforma.normativa.gestionar` | `NuevaAdministradoraPeticion` | 201 `AdministradoraRespuesta` | 403, 409, 422 |
| PUT | `/plataforma/nomina/administradoras/{administradora_id}` | Cambia el nombre y el estado de una administradora. | `plataforma.normativa.gestionar` | `CambiosAdministradoraPeticion` | 200 `AdministradoraRespuesta` | 403, 422 |
| POST | `/plataforma/nomina/tablas` | Agrega una versión completa de una tabla por tramos (retención por salarios o fondo de solidaridad). Nunca se edita: un error se corrige con otra versión. | `plataforma.normativa.gestionar` | `NuevaTablaLaboralPeticion` | 201 `TablaLaboralRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/suscripcion` | El plan de la entidad con la plataforma, su consumo del periodo en curso y sus facturas. | `suscripcion.ver` |  | 200 `MiSuscripcionRespuesta` | 403 |

## contabilidad

Contabilidad de cualquier entidad: configuración, plan de cuentas (PUC), terceros, periodos, comprobantes de partida doble, reglas, plantillas, reportes y estados financieros (`/entidades/{entidad_id}/contabilidad`)

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/organizaciones/{organizacion_id}/plantillas-contables` | Plantillas de la firma contable (la cuenta debe ser dueña o gestora). |  |  | 200 `[PlantillaResumenRespuesta]` |  |
| PUT | `/organizaciones/{organizacion_id}/plantillas-contables/{plantilla_id}/estado` | Publica o retira una plantilla de la firma contable. |  | `EstadoPlantillaPeticion` | 204  | 409 |
| GET | `/entidades/{entidad_id}` | Ficha de la entidad: nombre, NIT, ubicación, contacto y zona horaria (cualquier entidad). | `copropiedad.datos.ver` |  | 200 `FichaEntidadRespuesta` | 403 |
| PATCH | `/entidades/{entidad_id}` | Actualiza los datos generales de la entidad (en una PH, lo mismo que su ficha completa). | `copropiedad.datos.editar` | `CambiosEntidadPeticion` | 200 `FichaEntidadRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/capacidades` | Permisos de la cuenta en la entidad y módulos habilitados (para cualquier entidad). |  |  | 200 `CapacidadesRespuesta` |  |
| GET | `/entidades/{entidad_id}/terceros` | Terceros de la entidad (sus personas) con datos fiscales. | `contabilidad.ver` `facturacion.ver` `terceros.gestionar` |  | 200 `PaginaRespuesta_TerceroRespuesta` | 403 |
| POST | `/entidades/{entidad_id}/terceros` | Crea un tercero (reutiliza la persona si el documento ya existe). | `terceros.gestionar` | `NuevoTerceroPeticion` | 201 `TerceroRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/terceros/{persona_id}` | Un tercero por su persona (nombre, documento y datos fiscales): resuelve el `persona_id` de las líneas de un comprobante, de un recibo, de un egreso o de una factura. | `contabilidad.ver` `facturacion.ver` `terceros.gestionar` |  | 200 `TerceroRespuesta` | 403 |
| PUT | `/entidades/{entidad_id}/terceros/{persona_id}/datos-fiscales` | Reemplaza los datos fiscales de una persona de la entidad. |  | `DatosFiscalesDto` | 200 `DatosFiscalesDto` | 422 |
| GET | `/entidades/{entidad_id}/contabilidad/configuracion` | Configuración contable vigente (`null` si aún no la tiene). | `contabilidad.ver` |  | 200  | 403 |
| PUT | `/entidades/{entidad_id}/contabilidad/configuracion` | Guarda una versión nueva de la configuración contable. | `contabilidad.configurar` | `ConfiguracionPeticion` | 200 `ConfiguracionRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/puc` | Catálogo PUC de la plataforma (solo lectura). | `contabilidad.ver` |  | 200 `[CuentaPucDto]` | 403 |
| GET | `/entidades/{entidad_id}/contabilidad/cuentas` | Plan de cuentas de la entidad, por código. | `contabilidad.ver` |  | 200 `[CuentaRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/cuentas` | Crea una cuenta (su padre debe existir, salvo que sea una clase). | `contabilidad.configurar` | `NuevaCuentaPeticion` | 201 `CuentaRespuesta` | 403, 409, 422 |
| PUT | `/entidades/{entidad_id}/contabilidad/cuentas/{cuenta_id}` | Edita una cuenta (el código y la naturaleza no cambian). | `contabilidad.configurar` | `CambioCuentaPeticion` | 200 `CuentaRespuesta` | 403, 409 |
| GET | `/entidades/{entidad_id}/contabilidad/centros-costo` | Centros de costo. | `contabilidad.ver` |  | 200 `[CentroCostoRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/centros-costo` | Crea un centro de costo. | `contabilidad.configurar` | `CentroCostoPeticion` | 201 `CentroCostoRespuesta` | 403, 409 |
| PUT | `/entidades/{entidad_id}/contabilidad/centros-costo/{centro_id}` | Edita un centro de costo (nombre y estado). |  | `CentroCostoPeticion` | 200 `CentroCostoRespuesta` |  |
| GET | `/entidades/{entidad_id}/contabilidad/tipos-comprobante` | Tipos de comprobante. | `contabilidad.ver` |  | 200 `[TipoComprobanteRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/tipos-comprobante` | Crea un tipo de comprobante (su consecutivo es por año). | `contabilidad.configurar` | `TipoComprobantePeticion` | 201 `TipoComprobanteRespuesta` | 403, 409, 422 |
| PUT | `/entidades/{entidad_id}/contabilidad/tipos-comprobante/{tipo_id}` | Edita un tipo de comprobante (nombre, uso manual y estado). |  | `TipoComprobantePeticion` | 200 `TipoComprobanteRespuesta` |  |
| GET | `/entidades/{entidad_id}/contabilidad/reglas` | Reglas de contabilización (última versión de cada evento). | `contabilidad.ver` |  | 200 `[ReglaRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/reglas` | Crea una versión nueva de la regla de un evento. | `contabilidad.configurar` | `NuevaReglaPeticion` | 201 `ReglaRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/rubros` | Rubros de los estados financieros. | `contabilidad.ver` |  | 200 `[RubroDto]` | 403 |
| PUT | `/entidades/{entidad_id}/contabilidad/rubros` | Reemplaza los rubros de los estados financieros. |  | `[RubroDto]` | 200 `[RubroDto]` | 409, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/notas` | Notas a los estados financieros del año (última versión de cada una). | `contabilidad.ver` |  | 200 `[NotaRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/notas` | Guarda una versión nueva de una nota. | `contabilidad.configurar` | `NotaPeticion` | 201 `NotaRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/plantillas` | Plantillas que la cuenta puede aplicar (estándar y de sus firmas contables). | `contabilidad.configurar` |  | 200 `[PlantillaResumenRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/plantillas/aplicar` | Aplica una plantilla (o muestra qué haría con `previsualizar`). | `contabilidad.configurar` | `AplicarPlantillaPeticion` | 200 `ResumenAplicacionRespuesta` | 403, 422 |
| POST | `/entidades/{entidad_id}/contabilidad/plantillas/guardar` | Guarda la configuración de la entidad como plantilla en borrador (de su firma contable o, con el permiso de plataforma, estándar). | `contabilidad.configurar` | `GuardarPlantillaPeticion` | 201 `PlantillaRespuesta` | 403 |
| GET | `/entidades/{entidad_id}/contabilidad/periodos` | Periodos contables (del año o todos). | `contabilidad.ver` |  | 200 `[PeriodoRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/periodos` | Abre un periodo (si ya existe, lo devuelve como está). | `contabilidad.periodos.cerrar` | `AbrirPeriodoPeticion` | 200 `PeriodoRespuesta` | 403, 422 |
| PUT | `/entidades/{entidad_id}/contabilidad/periodos/{periodo_id}/estado` | Cambia el estado del periodo (revisión, cierre, o vuelta a abierto desde revisión). | `contabilidad.periodos.cerrar` | `EstadoPeriodoPeticion` | 200 `PeriodoRespuesta` | 403, 409 |
| GET | `/entidades/{entidad_id}/contabilidad/periodos/{periodo_id}/reaperturas` | Reaperturas de un periodo. | `contabilidad.ver` |  | 200 `[ReaperturaRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/periodos/{periodo_id}/reaperturas` | Solicita reabrir un periodo cerrado (la aprueba otra cuenta). | `contabilidad.periodos.reabrir` | `SolicitudReaperturaPeticion` | 201 `ReaperturaRespuesta` | 403, 409 |
| PUT | `/entidades/{entidad_id}/contabilidad/reaperturas/{reapertura_id}` | Aprueba o rechaza una reapertura (quien la solicitó no puede). | `contabilidad.periodos.aprobar_reapertura` | `DecisionReaperturaPeticion` | 200 `ReaperturaRespuesta` | 403, 409 |
| GET | `/entidades/{entidad_id}/contabilidad/comprobantes` | Comprobantes por fecha, más recientes primero (sin líneas; a lo sumo 500). | `contabilidad.ver` |  | 200 `[ComprobanteRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/comprobantes` | Guarda un comprobante manual en borrador (se valida al contabilizar). | `contabilidad.comprobantes.crear` | `BorradorPeticion` | 201 `ComprobanteRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/comprobantes/{comprobante_id}` | Comprobante con sus líneas. |  |  | 200 `ComprobanteRespuesta` |  |
| PUT | `/entidades/{entidad_id}/contabilidad/comprobantes/{comprobante_id}` | Reemplaza un borrador. |  | `BorradorPeticion` | 200 `ComprobanteRespuesta` | 409 |
| DELETE | `/entidades/{entidad_id}/contabilidad/comprobantes/{comprobante_id}` | Borra un borrador. |  |  | 204  | 409 |
| POST | `/entidades/{entidad_id}/contabilidad/comprobantes/previsualizar` | Valida un comprobante sin guardarlo y devuelve el asiento. |  | `BorradorPeticion` | 200 `AsientoRespuesta` | 422 |
| POST | `/entidades/{entidad_id}/contabilidad/comprobantes/{comprobante_id}/contabilizar` | Contabiliza un borrador (partida doble, cuentas y periodo abierto; asigna número). | `contabilidad.comprobantes.contabilizar` |  | 200 `ComprobanteRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/contabilidad/comprobantes/{comprobante_id}/revertir` | Revierte un comprobante contabilizado con otro de caras invertidas. | `contabilidad.comprobantes.revertir` | `RevertirPeticion` | 201 `ComprobanteRespuesta` | 403, 409 |
| POST | `/entidades/{entidad_id}/contabilidad/documentos/previsualizar` | Muestra el asiento que produciría un documento origen con la regla vigente (sin guardarlo). |  | `DocumentoContablePeticion` | 200 `AsientoRespuesta` | 422 |
| POST | `/entidades/{entidad_id}/contabilidad/saldos-iniciales` | Contabiliza los saldos iniciales en un comprobante de apertura. | `contabilidad.comprobantes.contabilizar` | `SaldosInicialesPeticion` | 201 `ComprobanteRespuesta` | 403, 422 |
| POST | `/entidades/{entidad_id}/contabilidad/cierre-anual` | Cierre anual: lleva a cero las cuentas de resultado contra la cuenta de excedente o déficit. | `contabilidad.comprobantes.contabilizar` | `CierreAnualPeticion` | 201 `ComprobanteRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/reportes/balance-prueba` | Balance de prueba entre dos fechas (por cuenta, tercero y centro de costo). | `contabilidad.ver` `contabilidad.exportar` |  | 200 `[FilaSaldoRespuesta]` | 403 |
| GET | `/entidades/{entidad_id}/contabilidad/reportes/libro-diario` | Libro diario: comprobantes contabilizados con sus líneas. | `contabilidad.ver` `contabilidad.exportar` |  | 200 `[ComprobanteRespuesta]` | 403 |
| GET | `/entidades/{entidad_id}/contabilidad/reportes/libro-mayor` | Libro mayor de una cuenta (o auxiliar de un tercero). |  |  | 200 `MayorRespuesta` |  |
| GET | `/entidades/{entidad_id}/contabilidad/reportes/estados-financieros` | Estados financieros (situación financiera y resultados) a la fecha de corte, con notas. | `contabilidad.ver` |  | 200 `EstadosFinancierosRespuesta` | 403 |

## tesoreria

Tesorería de cualquier entidad: cuentas de tesorería, recibos de caja, egresos con autorización de otra cuenta, facturas de proveedores con retenciones, facturas de venta y cartera, extractos y conciliación bancaria (`/entidades/{entidad_id}/contabilidad/...`). Cada documento se contabiliza con la regla de su evento; anular es revertir

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/entidades/{entidad_id}/contabilidad/tesoreria/cuentas` | Cuentas de tesorería. | `contabilidad.ver` |  | 200 `[CuentaTesoreriaRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/tesoreria/cuentas` | Crea una cuenta de tesorería respaldada por una cuenta contable de movimiento de naturaleza débito. Del número de cuenta solo se guardan los cuatro últimos dígitos. | `tesoreria.configurar` | `NuevaCuentaTesoreriaPeticion` | 201 `CuentaTesoreriaRespuesta` | 403, 409, 422 |
| PUT | `/entidades/{entidad_id}/contabilidad/tesoreria/cuentas/{cuenta_id}` | Cambia el nombre, la destinación o si la cuenta está activa. | `tesoreria.configurar` | `CambioCuentaTesoreriaPeticion` | 200 `CuentaTesoreriaRespuesta` | 403, 409 |
| GET | `/entidades/{entidad_id}/contabilidad/tesoreria/recibos` | Recibos de caja por fecha descendente (a lo sumo 500). | `contabilidad.ver` |  | 200 `[ReciboRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/tesoreria/recibos` | Registra y contabiliza un recibo de caja; lo que no se aplica a facturas queda sin aplicar. | `tesoreria.recibos.registrar` | `NuevoReciboPeticion` | 201 `ReciboRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/tesoreria/recibos/{recibo_id}` | Recibo de caja con sus aplicaciones. | `contabilidad.ver` |  | 200 `ReciboRespuesta` | 403 |
| GET | `/entidades/{entidad_id}/contabilidad/tesoreria/recibos/{recibo_id}/pdf` | El recibo de caja en PDF (para entregar a quien pagó): pagador, valor, medio, a qué se aplicó y el número de su comprobante; si está anulado, lo dice. | `contabilidad.ver` |  | 200 `application/pdf` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/tesoreria/recibos/{recibo_id}/aplicaciones` | Aplica lo que el recibo tiene sin aplicar a una factura de venta (si no tenía pagador, queda identificado con el cliente de la factura). | `tesoreria.recibos.registrar` | `AplicarReciboPeticion` | 200 `ReciboRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/contabilidad/tesoreria/recibos/{recibo_id}/anulacion` | Anula el recibo: revierte su comprobante y devuelve el saldo a las facturas. | `tesoreria.recibos.registrar` | `AnulacionPeticion` | 200 `ReciboRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/tesoreria/egresos` | Egresos por fecha descendente (a lo sumo 500). | `contabilidad.ver` |  | 200 `[EgresoRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/tesoreria/egresos` | Registra un egreso por autorizar (facturas de proveedor del beneficiario, gastos directos o ambos). | `tesoreria.egresos.registrar` | `NuevoEgresoPeticion` | 201 `EgresoRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/tesoreria/egresos/{egreso_id}` | Egreso. | `contabilidad.ver` |  | 200 `EgresoRespuesta` | 403 |
| GET | `/entidades/{entidad_id}/contabilidad/tesoreria/egresos/{egreso_id}/pdf` | El comprobante de egreso en PDF (para la firma del beneficiario): a quién se pagó, cuánto, qué facturas y gastos cubre, su estado y el número de su comprobante. | `contabilidad.ver` |  | 200 `application/pdf` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/tesoreria/egresos/{egreso_id}/autorizacion` | Autoriza un egreso que registró **otra** cuenta (segregación de funciones). | `tesoreria.egresos.autorizar` |  | 200 `EgresoRespuesta` | 403, 409 |
| POST | `/entidades/{entidad_id}/contabilidad/tesoreria/egresos/{egreso_id}/pago` | Registra el pago de un egreso autorizado: lo contabiliza y abona las facturas que paga. | `tesoreria.egresos.registrar` | `PagoEgresoPeticion` | 200 `EgresoRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/contabilidad/tesoreria/egresos/{egreso_id}/anulacion` | Anula el egreso; si ya se pagó, revierte su comprobante y devuelve el saldo a las facturas. | `tesoreria.egresos.registrar` | `AnulacionPeticion` | 200 `EgresoRespuesta` | 403, 409 |
| GET | `/entidades/{entidad_id}/contabilidad/tesoreria/retenciones` | Conceptos de retención del catálogo de la plataforma. | `contabilidad.ver` |  | 200 `[ConceptoRetencionRespuesta]` | 403 |
| GET | `/entidades/{entidad_id}/contabilidad/tesoreria/uvt` | Valores de la UVT por año. | `contabilidad.ver` |  | 200 `[UvtRespuesta]` | 403 |
| GET | `/entidades/{entidad_id}/contabilidad/cuentas-por-pagar/facturas` | Facturas de proveedor por vencimiento (a lo sumo 500, sin líneas). | `contabilidad.ver` |  | 200 `[FacturaProveedorRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/cuentas-por-pagar/facturas` | Registra una factura de proveedor y calcula sus retenciones con la tarifa vigente en su fecha. Un número que el proveedor ya usó se rechaza salvo excepción con motivo. | `tesoreria.facturas_proveedor.gestionar` `tesoreria.egresos.autorizar` | `NuevaFacturaProveedorPeticion` | 201 `FacturaProveedorRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/cuentas-por-pagar/facturas/{factura_id}` | Factura de proveedor con sus líneas y retenciones. | `contabilidad.ver` |  | 200 `FacturaProveedorRespuesta` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/cuentas-por-pagar/facturas/{factura_id}/causacion` | Causa la factura (gasto, IVA, retenciones y cuenta por pagar) en su fecha. | `tesoreria.facturas_proveedor.gestionar` |  | 200 `FacturaProveedorRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/contabilidad/cuentas-por-pagar/facturas/{factura_id}/anulacion` | Anula una factura sin pagos (si estaba causada, revierte su comprobante). | `tesoreria.facturas_proveedor.gestionar` | `AnulacionPeticion` | 200 `FacturaProveedorRespuesta` | 403, 409 |
| GET | `/entidades/{entidad_id}/contabilidad/cuentas-por-pagar/certificados/{persona_id}` | Certificado de las retenciones practicadas al tercero en el periodo. | `tesoreria.facturas_proveedor.gestionar` |  | 200 `CertificadoRetencionRespuesta` | 403 |
| GET | `/entidades/{entidad_id}/contabilidad/cuentas-por-pagar/certificados/{persona_id}/pdf` | El certificado de retenciones en PDF. | `tesoreria.facturas_proveedor.gestionar` |  | 200 `application/pdf` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/cuentas-por-pagar/certificados/{persona_id}/envio` | Envía el certificado por correo al correo de facturación del tercero (o a su correo). | `tesoreria.facturas_proveedor.gestionar` |  | 202 `EnvioCertificadoRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/cuentas-por-pagar/edades` | Cuentas por pagar por edades a la fecha de corte: saldo de cada factura de proveedor (neto a pagar − pagado) en el tramo de sus días vencidos, por proveedor. | `contabilidad.ver` |  | 200 `[FilaCarteraRespuesta]` | 403 |
| GET | `/entidades/{entidad_id}/contabilidad/cuentas-por-cobrar/edades` | Cartera de clientes por edades a la fecha de corte (saldo actual de cada factura en el tramo de sus días vencidos). | `contabilidad.ver` |  | 200 `[FilaCarteraRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/tesoreria/cuentas/{cuenta_id}/extractos/vista-previa` | Lee el extracto y muestra lo que se importaría, con los problemas por fila (no guarda nada). | `tesoreria.conciliar` | `ExtractoPeticion` | 200 `VistaPreviaExtractoRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/tesoreria/cuentas/{cuenta_id}/mapeo-extracto` | Último mapeo de columnas de la cuenta (se guarda solo al importar un extracto en planilla): prellene con él el formulario de carga. | `tesoreria.conciliar` |  | 200 `MapeoExtractoRespuesta` | 403 |
| PUT | `/entidades/{entidad_id}/contabilidad/tesoreria/cuentas/{cuenta_id}/mapeo-extracto` | Guarda el mapeo de columnas de la cuenta (el del banco de esa cuenta). | `tesoreria.conciliar` | `MapeoColumnasDto` | 200 `MapeoExtractoRespuesta` | 403 |
| GET | `/entidades/{entidad_id}/contabilidad/tesoreria/cuentas/{cuenta_id}/extractos` | Extractos importados en la cuenta, más recientes primero. | `contabilidad.ver` |  | 200 `[ExtractoRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/tesoreria/cuentas/{cuenta_id}/extractos` | Importa el extracto (el archivo queda como soporte). El mismo archivo no se importa dos veces; un extracto con filas inválidas se rechaza (revíselo con la vista previa). | `tesoreria.conciliar` | `ExtractoPeticion` | 201 `ExtractoRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/tesoreria/cuentas/{cuenta_id}/movimientos` | Movimientos del extracto de la cuenta entre dos fechas. | `contabilidad.ver` |  | 200 `[MovimientoExtractoRespuesta]` | 403 |
| GET | `/entidades/{entidad_id}/contabilidad/tesoreria/cuentas/{cuenta_id}/libros` | Líneas contabilizadas de la cuenta contable del banco entre dos fechas. | `contabilidad.ver` |  | 200 `[LineaLibrosRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/tesoreria/cuentas/{cuenta_id}/conciliacion-automatica` | Concilia automáticamente por valor, fecha (con tolerancia) y referencia; lo que no casa queda para la conciliación manual. | `tesoreria.conciliar` | `ConciliacionAutomaticaPeticion` | 200 `[ConciliacionRespuesta]` | 403, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/tesoreria/cuentas/{cuenta_id}/conciliaciones` | Conciliaciones de la cuenta, más recientes primero. | `contabilidad.ver` |  | 200 `[ConciliacionRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/tesoreria/cuentas/{cuenta_id}/conciliaciones` | Concilia a mano movimientos y líneas de libros cuyas sumas coinciden. | `tesoreria.conciliar` | `ConciliacionManualPeticion` | 201 `ConciliacionRespuesta` | 403, 422 |
| POST | `/entidades/{entidad_id}/contabilidad/tesoreria/conciliaciones/{conciliacion_id}/deshacer` | Deshace una conciliación con un registro nuevo (sus movimientos vuelven a pendientes). | `tesoreria.conciliar` |  | 201 `ConciliacionRespuesta` | 403, 409 |
| PUT | `/entidades/{entidad_id}/contabilidad/tesoreria/movimientos/{movimiento_id}/partida-conciliatoria` | Marca un movimiento pendiente como partida conciliatoria (o lo devuelve a pendiente). | `tesoreria.conciliar` | `PartidaConciliatoriaPeticion` | 200 `MovimientoExtractoRespuesta` | 403, 409 |
| POST | `/entidades/{entidad_id}/contabilidad/tesoreria/movimientos/{movimiento_id}/recibo` | Crea un recibo no identificado por un crédito del extracto y lo concilia con él. | `tesoreria.conciliar` `tesoreria.recibos.registrar` | `ReciboNoIdentificadoPeticion` | 201 `ReciboRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/tesoreria/cuentas/{cuenta_id}/informe-conciliacion` | Informe de conciliación: saldo del extracto, de libros y partidas conciliatorias. | `contabilidad.ver` |  | 200 `InformeConciliacionRespuesta` | 403 |

## cartera

Cartera de la propiedad horizontal (módulo `contabilidad_ph`, `/copropiedades/{copropiedad_id}/cartera/...`): conceptos de cobro, políticas de cartera, módulos de contribución, presupuesto y su ejecución, cuotas extraordinarias, liquidación de expensas con cuentas de cobro, recaudo por bien con imputación y pronto pago, notas crédito, cartera inicial, estado de cuenta y cuadre contra el mayor

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/copropiedades/{copropiedad_id}/cartera/modulos-contribucion` | Módulos de contribución por nombre. | `cartera.ver` |  | 200 `[ModuloContribucionRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cartera/modulos-contribucion` | Crea un módulo de contribución (conjuntos mixtos, art. 29). Sus coeficientes se cargan como un cuadro de coeficientes con `modulo_id`. | `cartera.configurar` | `ModuloContribucionPeticion` | 201 `ModuloContribucionRespuesta` | 403, 409, 422 |
| PUT | `/copropiedades/{copropiedad_id}/cartera/modulos-contribucion/{modulo_id}` | Actualiza un módulo de contribución. | `cartera.configurar` | `ModuloContribucionPeticion` | 200 `ModuloContribucionRespuesta` | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/cartera/conceptos` | Conceptos de cobro por código. | `cartera.ver` |  | 200 `[ConceptoCobroRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cartera/conceptos` | Crea un concepto de cobro (su `concepto_contable` es el de las reglas `ph.*`). | `cartera.configurar` | `NuevoConceptoPeticion` | 201 `ConceptoCobroRespuesta` | 403, 409, 422 |
| PUT | `/copropiedades/{copropiedad_id}/cartera/conceptos/{concepto_id}` | Actualiza un concepto de cobro (código, tipo y criterio no cambian). | `cartera.configurar` | `CambioConceptoPeticion` | 200 `ConceptoCobroRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/cartera/politicas` | Versiones de la política de cartera, la más reciente primero. | `cartera.ver` |  | 200 `[PoliticaCarteraRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cartera/politicas` | Crea una versión de la política de cartera (vencimiento, redondeo, pronto pago, mora, imputación y fondo de imprevistos). | `cartera.configurar` | `NuevaPoliticaPeticion` | 201 `PoliticaCarteraRespuesta` | 403, 409, 422 |
| GET | `/copropiedades/{copropiedad_id}/cartera/bienes` | Bienes privados con su referencia de pago, pagador y deudor. | `cartera.ver` |  | 200 `[BienCarteraRespuesta]` | 403 |
| PUT | `/copropiedades/{copropiedad_id}/cartera/bienes/{bien_privado_id}/datos-pago` | Fija la referencia de pago y el pagador (tenedor) de un bien. | `cartera.configurar` | `DatosPagoPeticion` | 200 `BienCarteraRespuesta` | 403, 409, 422 |
| GET | `/copropiedades/{copropiedad_id}/cartera/presupuestos` | Presupuestos (del año, si se da), del más reciente al más antiguo. | `cartera.ver` |  | 200 `[PresupuestoRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cartera/presupuestos` | Crea un presupuesto en borrador (siguiente versión del año). | `presupuesto.gestionar` | `NuevoPresupuestoPeticion` | 201 `PresupuestoRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/cartera/presupuestos/{presupuesto_id}` | Presupuesto con sus rubros. | `cartera.ver` |  | 200 `PresupuestoRespuesta` | 403 |
| PUT | `/copropiedades/{copropiedad_id}/cartera/presupuestos/{presupuesto_id}/rubros` | Reemplaza los rubros de un borrador. | `presupuesto.gestionar` | `RubrosPeticion` | 200 `PresupuestoRespuesta` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/cartera/presupuestos/{presupuesto_id}/aprobacion` | Aprueba el borrador (asamblea): pasa a vigente y reemplaza la versión vigente del año. | `presupuesto.gestionar` | `AprobacionPresupuestoPeticion` | 200 `PresupuestoRespuesta` | 403, 409, 422 |
| GET | `/copropiedades/{copropiedad_id}/cartera/presupuestos/{presupuesto_id}/ejecucion` | Ejecución presupuestal: lo presupuestado contra el mayor, de enero al mes del corte. | `cartera.ver` |  | 200 `EjecucionPresupuestalRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/cartera/cuotas-extraordinarias` | Cuotas extraordinarias, las más recientes primero. | `cartera.ver` |  | 200 `[CuotaExtraordinariaRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cartera/cuotas-extraordinarias` | Registra una cuota extraordinaria aprobada: se cobra en las liquidaciones de su calendario. | `cartera.liquidar` | `NuevaCuotaPeticion` | 201 `CuotaExtraordinariaRespuesta` | 403, 422 |
| POST | `/copropiedades/{copropiedad_id}/cartera/cuotas-extraordinarias/{cuota_id}/anulacion` | Anula una cuota extraordinaria: las liquidaciones siguientes ya no la cobran. | `cartera.liquidar` |  | 204  | 403 |
| POST | `/copropiedades/{copropiedad_id}/cartera/liquidaciones/vista-previa` | Calcula la liquidación del periodo sin guardar nada. | `cartera.liquidar` | `PeriodoPeticion` | 200 `VistaLiquidacionRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/cartera/liquidaciones` | Liquidaciones por periodo descendente. | `cartera.ver` |  | 200 `[LiquidacionCarteraRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cartera/liquidaciones` | Liquida y contabiliza el periodo: obligaciones de cada bien, comprobante (evento `ph.liquidacion`), una cuenta de cobro por bien y la aplicación de los anticipos. | `cartera.liquidar` | `PeriodoPeticion` | 201 `LiquidadoRespuesta` | 403, 409, 422 |
| GET | `/copropiedades/{copropiedad_id}/cartera/liquidaciones/{liquidacion_id}` | Liquidación con sus obligaciones y su saldo actual. | `cartera.ver` |  | 200 `DetalleLiquidacionRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cartera/liquidaciones/{liquidacion_id}/anulacion` | Anula una liquidación sin pagos ni ajustes (revierte su comprobante; los cargos de reservas vuelven a pendientes). Con pagos se corrige con notas crédito. | `cartera.liquidar` | `MotivoCarteraPeticion` | 200 `LiquidacionCarteraRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/cartera/liquidaciones/{liquidacion_id}/envio` | Envía por correo cada cuenta de cobro de la liquidación a su pagador. | `cartera.liquidar` |  | 200 `EnvioCuentasRespuesta` | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/cartera/cuentas-cobro` | Cuentas de cobro de una liquidación o de un bien, por número. | `cartera.ver` |  | 200 `[CuentaCobroRespuesta]` | 403 |
| GET | `/copropiedades/{copropiedad_id}/cartera/cuentas-cobro/{cuenta_cobro_id}/pdf` | La cuenta de cobro en PDF. | `cartera.ver` |  | 200 `application/pdf` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cartera/recaudos` | Registra un pago para un bien: se imputa a sus obligaciones según la política, gana el pronto pago si llega a tiempo y cubre la deuda, y lo que sobra queda como anticipo. | `tesoreria.recibos.registrar` | `NuevoRecaudoPeticion` | 201 `RecaudoRespuesta` | 403, 422 |
| POST | `/copropiedades/{copropiedad_id}/cartera/recaudos/por-referencia` | Recauda los créditos pendientes del extracto cuya referencia es la de un bien (archivo Asobancaria o extracto con referencia) y los concilia. | `tesoreria.conciliar` `tesoreria.recibos.registrar` | `RecaudoReferenciaPeticion` | 200 `RecaudoExtractoRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cartera/recaudos/{recibo_id}/anulacion` | Anula un recaudo: revierte su pronto pago y anula el recibo (las obligaciones recuperan su saldo). | `tesoreria.recibos.registrar` | `MotivoCarteraPeticion` | 200 `ReciboRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/cartera/recibos/{recibo_id}/aplicacion` | Aplica lo sin aplicar de un recibo (anticipo o por identificar) a las obligaciones de un bien. | `tesoreria.recibos.registrar` | `AplicacionBienPeticion` | 200 `[AplicadoRespuesta]` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/cartera/cobros` | Registra un cobro manual (multa, uso de un bien común, daño, otro) con su comprobante (evento `ph.cobro`). | `cartera.ajustar` | `NuevoCobroPeticion` | 201 `ObligacionRespuesta` | 403, 422 |
| POST | `/copropiedades/{copropiedad_id}/cartera/obligaciones/{obligacion_id}/anulacion` | Anula un cobro manual sin pagos ni ajustes (revierte su comprobante). | `cartera.ajustar` | `MotivoCarteraPeticion` | 204  | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/cartera/obligaciones/{obligacion_id}/notas-credito` | Nota crédito sobre una obligación (corrige lo liquidado; evento `ph.nota_credito`). | `cartera.ajustar` | `NotaCreditoPeticion` | 201 `AjusteRespuesta` | 403, 422 |
| POST | `/copropiedades/{copropiedad_id}/cartera/inicial` | Carga (o valida) la cartera inicial por bien, concepto y antigüedad. Con errores no carga nada. | `cartera.ajustar` | `CarteraInicialPeticion` | 200 `CarteraInicialRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/cartera/bienes/{bien_privado_id}/estado-cuenta` | Estado de cuenta de un bien al corte. | `cartera.ver` |  | 200 `EstadoCuentaRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/cartera/saldos` | Cartera por bien al corte (solo los bienes con saldo). | `cartera.ver` |  | 200 `[SaldoBienRespuesta]` | 403 |
| GET | `/copropiedades/{copropiedad_id}/cartera/cuadre` | Cuadre de la cartera contra el mayor (cuentas que debitan las reglas `ph.liquidacion`, `ph.cobro` y `ph.intereses`). | `cartera.ver` |  | 200 `[CuadreCuentaRespuesta]` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/cartera/tasas-referencia` | Tasas de referencia por vigencia (las que usa la causación de intereses). | `cartera.ver` |  | 200 `[TasaReferenciaRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cartera/intereses/vista-previa` | Calcula los intereses de mora hasta la fecha sin guardar nada. | `cartera.liquidar` | `HastaPeticion` | 200 `CalculoMoraRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/cartera/intereses` | Causaciones de intereses por periodo descendente. | `cartera.ver` |  | 200 `[CausacionRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cartera/intereses` | Causa y contabiliza los intereses de mora hasta la fecha (evento `ph.intereses`). | `cartera.liquidar` | `HastaPeticion` | 201 `CausadoRespuesta` | 403, 409, 422 |
| GET | `/copropiedades/{copropiedad_id}/cartera/intereses/{causacion_id}` | Causación con el detalle de cada tramo (base, días y tasa). | `cartera.ver` |  | 200 `DetalleCausacionRespuesta` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cartera/intereses/{causacion_id}/anulacion` | Anula una causación sin pagos ni ajustes sobre sus intereses. | `cartera.liquidar` | `MotivoCarteraPeticion` | 200 `CausacionRespuesta` | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/cartera/acuerdos` | Acuerdos de pago (del bien, si se da) con su situación hoy. | `cartera.ver` |  | 200 `[AcuerdoRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cartera/acuerdos` | Propone un acuerdo de pago por todo lo que debe el bien (lo aprueba otra cuenta). | `cartera.ajustar` | `NuevoAcuerdoPeticion` | 201 `AcuerdoRespuesta` | 403, 409, 422 |
| POST | `/copropiedades/{copropiedad_id}/cartera/acuerdos/{acuerdo_id}/aprobacion` | Aprueba un acuerdo propuesto por otra cuenta (suspende la mora). | `cartera.liquidar` |  | 200 `AcuerdoRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/cartera/acuerdos/{acuerdo_id}/anulacion` | Anula un acuerdo propuesto o vigente (la mora se reanuda). | `cartera.liquidar` |  | 200 `AcuerdoRespuesta` | 403, 409 |
| GET | `/copropiedades/{copropiedad_id}/cartera/edades` | Cartera por edades (0–30, 31–60, 61–90, más de 90 días) por bien, con el total. | `cartera.ver` |  | 200 `CarteraEdadesRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/cartera/bienes/{bien_privado_id}/certificados` | Certificados emitidos de un bien. | `cartera.ver` |  | 200 `[CertificadoRespuesta]` | 403 |
| POST | `/copropiedades/{copropiedad_id}/cartera/bienes/{bien_privado_id}/certificados` | Emite un paz y salvo (solo si el bien no debe) o un certificado de deuda. El código de verificación y el PDF se entregan una sola vez. | `cartera.certificados.emitir` | `NuevoCertificadoPeticion` | 201 `CertificadoEmitidoRespuesta` | 403, 409 |
| POST | 🔓 `/copropiedades/{copropiedad_id}/certificados/verificacion` | Verifica el código de un paz y salvo o certificado de deuda (sin sesión: el código es la credencial; límite por IP). Solo dice si es válido, su clase y su corte. |  | `VerificacionPeticion` | 200 `VerificacionRespuesta` | 429 |
| GET | `/copropiedades/{copropiedad_id}/mi-cartera/bienes` | Bienes de la cuenta (titular vigente o pagador) con sus datos de pago. | _No exige permisos (solo bienes de los que la cuenta es titular o pagador); módulo no contratado (`modulo_no_habilitado`)_ |  | 200 `[BienCarteraRespuesta]` | 403 |
| GET | `/copropiedades/{copropiedad_id}/mi-cartera/bienes/{bien_privado_id}/estado-cuenta` | Estado de cuenta de un bien propio. | _No exige permisos (si la cuenta no es titular ni pagador del bien, 404); módulo no contratado (`modulo_no_habilitado`)_ |  | 200 `EstadoCuentaRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/mi-cartera/bienes/{bien_privado_id}/cuentas-cobro` | Cuentas de cobro de un bien propio. | _No exige permisos (si la cuenta no es titular ni pagador del bien, 404); módulo no contratado (`modulo_no_habilitado`)_ |  | 200 `[CuentaCobroRespuesta]` | 403 |
| GET | `/copropiedades/{copropiedad_id}/mi-cartera/cuentas-cobro/{cuenta_cobro_id}/pdf` | PDF de una cuenta de cobro de un bien propio. | _No exige permisos (si la cuenta no es titular ni pagador del bien, 404); módulo no contratado (`modulo_no_habilitado`)_ |  | 200 `application/pdf` | 403 |
| POST | `/copropiedades/{copropiedad_id}/mi-cartera/bienes/{bien_privado_id}/paz-y-salvo` | El propietario o pagador descarga el paz y salvo de su bien (solo si no debe nada). El código y el PDF se entregan una sola vez. | _No exige permisos (si la cuenta no es titular ni pagador del bien, 404); módulo no contratado (`modulo_no_habilitado`)_ |  | 201 `CertificadoEmitidoRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/cartera/asambleas/{reunion_id}/morosos` | Fija los morosos de una reunión con los bienes que tienen cartera vencida al corte. | `cartera.ver` `asambleas.convocar` | `CorteMorososPeticion` | 200 `MorososRespuesta` | 403, 409 |
| POST | `/copropiedades/{copropiedad_id}/cartera/reservas/restricciones-mora` | Restringe las reservas de los bienes con vencido desde el umbral y levanta las que ya no lo superan (solo las que creó la cartera). | `cartera.ver` `reservas.restringir` | `UmbralMoraPeticion` | 200 `SincronizacionMoraRespuesta` | 403 |
| GET | `/copropiedades/{copropiedad_id}/cartera/informes/fondo` | Estado del fondo de imprevistos frente al presupuesto ordinario del año. | `cartera.ver` |  | 200 `EstadoFondoRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/cartera/informes/flujo-caja` | Flujo de caja mensual de las cuentas de tesorería (de 1 a 24 meses). | `cartera.ver` |  | 200 `[MesFlujoRespuesta]` | 403, 422 |
| POST | `/copropiedades/{copropiedad_id}/cartera/presupuestos/proyecto` | Proyecto de presupuesto del año siguiente: copia el vigente con un ajuste porcentual (queda en borrador). | `presupuesto.gestionar` | `ProyectoPresupuestoPeticion` | 201 `PresupuestoRespuesta` | 403, 422 |
| GET | `/copropiedades/{copropiedad_id}/cartera/informes/asamblea` | Paquete en PDF para la asamblea: ejecución presupuestal, cartera por edades y fondo. | `cartera.ver` |  | 200 `application/pdf` | 403, 422 |

## tributario

Tributario de cualquier entidad (plan 15): catálogos de IVA, ICA y exógena, perfil tributario versionado que confirma el contador, resoluciones de numeración, documentos electrónicos (factura, nota crédito, documento soporte) con su proveedor, facturas recibidas por XML con eventos RADIAN, preparación de retención, IVA e ICA, información exógena y beneficiarios finales. El sistema prepara; presentar y firmar lo hace el responsable tributario

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/entidades/{entidad_id}/tributario/catalogo` | Catálogo tributario de la plataforma (tarifas de IVA e ICA y formatos de exógena). | `contabilidad.ver` `facturacion.ver` |  | 200 `CatalogoTributarioRespuesta` | 403 |
| GET | `/entidades/{entidad_id}/tributario/perfiles` | Versiones del perfil tributario (la más reciente primero). | `contabilidad.ver` `facturacion.ver` |  | 200 `[PerfilTributarioRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/tributario/perfiles` | Crea una versión del perfil en borrador (no aplica nada hasta que el contador la confirme). | `tributario.configurar` | `NuevoPerfilPeticion` | 201 `PerfilTributarioRespuesta` | 403, 422 |
| POST | `/entidades/{entidad_id}/tributario/perfiles/{perfil_id}/confirmacion` | Confirma una versión del perfil: desde su vigencia aplican IVA, retenciones y facturación. Una versión confirmada no se modifica. | `tributario.perfil.confirmar` |  | 200 `PerfilTributarioRespuesta` | 403, 409 |
| GET | `/entidades/{entidad_id}/tributario/resoluciones` | Resoluciones de numeración (sin la clave técnica). | `contabilidad.ver` `facturacion.ver` |  | 200 `[ResolucionRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/tributario/resoluciones` | Registra una resolución de numeración de la DIAN. | `tributario.configurar` | `NuevaResolucionPeticion` | 201 `ResolucionRespuesta` | 403, 409, 422 |
| PUT | `/entidades/{entidad_id}/tributario/resoluciones/{resolucion_id}/activa` | Activa o desactiva una resolución (solo una activa y vigente numera). | `tributario.configurar` | `ActivaPeticion` | 204  | 403 |
| GET | `/entidades/{entidad_id}/contabilidad/tributario/retencion` | Prepara la retención en la fuente del periodo con su conciliación contra el mayor (no se presenta desde aquí). | `tributario.preparar` |  | 200 `PreparacionRetencionRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/tributario/iva` | Prepara el IVA del periodo: generado y descontable por tarifa. | `tributario.preparar` |  | 200 `PreparacionIvaRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/tributario/ica` | Prepara el ICA de los municipios y actividades del perfil confirmado y la ReteICA. | `tributario.preparar` |  | 200 `PreparacionIcaRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/contabilidad/tributario/exogena` | Generaciones de exógena (las más recientes primero). | `tributario.preparar` |  | 200 `[GeneracionExogenaRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/tributario/exogena` | Genera un formato de exógena con la versión de reglas del catálogo (cuantías menores en el NIT 222222222) y guarda el archivo con su huella. | `tributario.preparar` | `GenerarExogenaPeticion` | 201 `ExogenaGeneradaRespuesta` | 403 |
| GET | `/entidades/{entidad_id}/contabilidad/tributario/beneficiarios` | Beneficiarios finales (los vigentes primero). | `tributario.preparar` |  | 200 `[BeneficiarioRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/contabilidad/tributario/beneficiarios` | Registra un beneficiario final. | `tributario.preparar` | `NuevoBeneficiarioPeticion` | 201 `BeneficiarioRespuesta` | 403, 422 |
| POST | `/entidades/{entidad_id}/contabilidad/tributario/beneficiarios/{beneficiario_id}/terminacion` | Termina la vigencia de un beneficiario final. | `tributario.preparar` | `TerminarPeticion` | 204  | 403 |

## facturacion

Facturación de cualquier entidad, con o sin contabilidad (plan 18): catálogo de productos y servicios, facturas de venta con número interno, pago manual sin tesorería, documentos electrónicos, compras para el documento soporte, facturas recibidas, libros de ventas y compras y paquete de XML y PDF para el contador. Con contabilidad cada documento se contabiliza solo

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| GET | `/entidades/{entidad_id}/facturacion/items` | Catálogo por descripción (a lo sumo 500). | `facturacion.ver` |  | 200 `[ItemRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/facturacion/items` | Crea un producto o servicio. | `facturacion.items.gestionar` | `NuevoItemPeticion` | 201 `ItemRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/facturacion/items/{item_id}` | Producto o servicio por id. | `facturacion.ver` |  | 200 `ItemRespuesta` | 403 |
| PUT | `/entidades/{entidad_id}/facturacion/items/{item_id}` | Reemplaza los datos de un ítem (el código no cambia). Desactivarlo lo saca de las facturas nuevas. | `facturacion.items.gestionar` | `DatosItemPeticion` | 200 `ItemRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/facturacion/facturas` | Facturas de venta por vencimiento (a lo sumo 500, sin líneas). | `facturacion.ver` |  | 200 `[FacturaVentaRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/facturacion/facturas` | Crea una factura de venta en borrador (el IVA de cada línea lo elige quien factura). | `facturacion.facturas.gestionar` | `NuevaFacturaVentaPeticion` | 201 `FacturaVentaRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/facturacion/facturas/{factura_id}` | Factura de venta con sus líneas. | `facturacion.ver` |  | 200 `FacturaVentaRespuesta` | 403 |
| DELETE | `/entidades/{entidad_id}/facturacion/facturas/{factura_id}` | Borra una factura en borrador. | `facturacion.facturas.gestionar` |  | 204  | 403, 409 |
| POST | `/entidades/{entidad_id}/facturacion/facturas/{factura_id}/emision` | Emite la factura: le asigna su número interno del año y, si la entidad lleva contabilidad, la contabiliza. | `facturacion.facturas.gestionar` |  | 200 `FacturaVentaRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/facturacion/facturas/{factura_id}/anulacion` | Anula una factura emitida sin pagos (revierte su comprobante si se contabilizó). Con factura electrónica validada se anula con nota crédito. | `facturacion.facturas.gestionar` | `AnulacionPeticion` | 200 `FacturaVentaRespuesta` | 403, 409 |
| POST | `/entidades/{entidad_id}/facturacion/facturas/{factura_id}/pago` | Marca pagada una factura emitida de una entidad **sin contabilidad**. Con contabilidad el pago se registra con un recibo de caja de la tesorería. | `facturacion.facturas.gestionar` | `PagoManualPeticion` | 200 `FacturaVentaRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/facturacion/facturas/{factura_id}/nota-debito` | Emite la nota débito de una factura con factura electrónica validada: un documento por cobrar más del mismo cliente (entra a la cartera, se abona con recibos de caja o se marca pagada a mano y, con contabilidad, se contabiliza) que se transmite a la DIAN referido a la factura electrónica original. | `facturacion.facturas.gestionar` `facturacion.emitir` | `NotaDebitoPeticion` | 200 `DocumentoElectronicoRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/facturacion/facturas/{factura_id}/factura-electronica` | Emite la factura electrónica de una factura de venta emitida. Pedirla otra vez devuelve el mismo documento (mismo CUFE) y lo reintenta si quedó en contingencia. | `facturacion.emitir` |  | 200 `DocumentoElectronicoRespuesta` | 403, 422 |
| POST | `/entidades/{entidad_id}/facturacion/facturas/{factura_id}/factura-contingencia` | Transcribe como factura electrónica **tipo 03** una factura de venta que se expidió en papel con el talonario de contingencia del facturador mientras no pudo facturar electrónicamente. La DIAN exige transcribirla dentro de las 48 horas siguientes a superar el inconveniente. | `facturacion.emitir` | `TranscripcionPeticion` | 200 `DocumentoElectronicoRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/facturacion/facturas/{factura_id}/nota-credito` | Anula una factura de venta con factura electrónica validada mediante una nota crédito electrónica (revierte su contabilización). | `facturacion.emitir` | `NotaCreditoVentaPeticion` | 200 `DocumentoElectronicoRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/facturacion/compras/{factura_id}/documento-soporte` | Emite el documento soporte de una compra a quien no está obligado a facturar. | `facturacion.emitir` |  | 200 `DocumentoElectronicoRespuesta` | 403, 422 |
| POST | `/entidades/{entidad_id}/facturacion/compras/{factura_id}/nota-ajuste` | Emite la nota de ajuste que anula el documento soporte validado de una compra (anula también la compra y, con contabilidad, revierte su comprobante). | `facturacion.facturas.gestionar` `facturacion.emitir` | `NotaCreditoVentaPeticion` | 200 `DocumentoElectronicoRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/facturacion/documentos` | Documentos electrónicos (los más recientes primero, a lo sumo 500). | `facturacion.ver` |  | 200 `[DocumentoElectronicoRespuesta]` | 403 |
| GET | `/entidades/{entidad_id}/facturacion/documentos/{documento_id}` | Documento electrónico con sus eventos. | `facturacion.ver` |  | 200 `DocumentoDetalleRespuesta` | 403 |
| POST | `/entidades/{entidad_id}/facturacion/documentos/{documento_id}/reintento` | Reintenta un documento por enviar o en contingencia (o consulta uno enviado). | `facturacion.emitir` |  | 200 `DocumentoElectronicoRespuesta` | 403, 409 |
| GET | `/entidades/{entidad_id}/facturacion/documentos/{documento_id}/pdf` | Representación gráfica del documento en PDF (con su CUFE). | `facturacion.ver` |  | 200 `application/pdf` | 403 |
| GET | `/entidades/{entidad_id}/facturacion/documentos/{documento_id}/paquete` | ZIP del documento tal como lo recibe el adquirente: el `AttachedDocument` (el documento y la respuesta de la DIAN) y la representación gráfica en PDF. | `facturacion.ver` |  | 200 `application/zip` | 403 |
| GET | `/entidades/{entidad_id}/facturacion/recibidas` | Facturas electrónicas recibidas (las más recientes primero). | `facturacion.ver` |  | 200 `[FacturaRecibidaRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/facturacion/recibidas` | Carga el XML de una factura electrónica recibida: lee sus datos, la asocia al proveedor con ese NIT y la deja pendiente de registrar. La misma factura (CUFE o archivo) no entra dos veces. | `facturacion.recibir` | `CargarFacturaPeticion` | 201 `FacturaRecibidaRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/facturacion/recibidas/{recibida_id}` | Factura recibida con sus eventos RADIAN. | `facturacion.ver` |  | 200 `FacturaRecibidaDetalleRespuesta` | 403 |
| POST | `/entidades/{entidad_id}/facturacion/recibidas/{recibida_id}/vinculo` | Vincula la factura recibida con la factura de proveedor registrada (mismo CUFE). | `facturacion.recibir` | `VincularPeticion` | 200 `FacturaRecibidaRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/facturacion/recibidas/{recibida_id}/rechazo` | Rechaza internamente una factura recibida (no la registra). Para reclamarla ante el emisor se registra el evento RADIAN `reclamo`. | `facturacion.recibir` | `RechazoDocumentoPeticion` | 200 `FacturaRecibidaRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/facturacion/recibidas/{recibida_id}/eventos` | Registra un evento RADIAN (acuse, recibo del bien, aceptación expresa o reclamo) sobre una factura recibida. Con el canal `propio` se firma y se envía a la DIAN; si no, lo registra el proveedor. Cada evento exige sus previos y no se repite. | `facturacion.recibir` | `EventoRadianPeticion` | 201 `EventoRadianRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/facturacion/documentos/{documento_id}/aceptacion-tacita` | Registra la aceptación tácita (evento 034) de una factura electrónica propia, validada y a crédito, cuando el adquirente no la aceptó ni la reclamó en los tres días hábiles siguientes al recibo del bien. La DIAN verifica el plazo y los eventos del adquirente. | `facturacion.emitir` |  | 201 `EventoRadianRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/facturacion/documentos/{documento_id}/eventos` | Eventos RADIAN que la entidad registró sobre su propia factura electrónica. | `facturacion.ver` |  | 200 `[EventoRadianRespuesta]` | 403 |
| GET | `/entidades/{entidad_id}/facturacion/libro-ventas` | Libro de ventas del periodo en CSV (número interno, número y CUFE de la DIAN, cliente y valores). Queda en la auditoría con su huella (cabecera `X-Contenido-Sha256`). | `facturacion.exportar` |  | 200 `text/csv` | 403, 422 |
| GET | `/entidades/{entidad_id}/facturacion/libro-compras` | Libro de compras del periodo en CSV (facturas electrónicas recibidas). | `facturacion.exportar` |  | 200 `text/csv` | 403, 422 |
| GET | `/entidades/{entidad_id}/facturacion/paquete` | ZIP para conservar los documentos electrónicos del periodo: en `emitidos/` el XML firmado, el `AttachedDocument` y el PDF de cada documento transmitido; en `recibidos/` el XML de cada factura recibida; en `eventos/` los eventos RADIAN firmados; e `indice.csv` con el documento, el CUFE, el estado, la fecha y el SHA-256 de cada archivo. | `facturacion.exportar` |  | 200 `application/zip` | 403, 422 |
| POST | `/entidades/{entidad_id}/facturacion/contabilizacion` | Contabiliza las facturas emitidas antes de tener contabilidad (exige el módulo de contabilidad). Cada una va en su transacción; repetir no duplica nada. | `contabilidad.comprobantes.contabilizar` | `PeriodoParametros` | 200 `InformeContabilizacionRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/facturacion/habilitacion` | Estado de la habilitación: software, certificado y set de pruebas. | `facturacion.configurar` |  | 200 `HabilitacionRespuesta` | 403 |
| PUT | `/entidades/{entidad_id}/facturacion/habilitacion/software` | Registra (o cambia) el software de la entidad; cambiarlo vuelve la habilitación a `registrado`. | `facturacion.configurar` | `SoftwarePeticion` | 200 `HabilitacionRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/facturacion/habilitacion/certificado` | Carga el certificado de firma de la entidad (lo valida, lo cifra y lo deja activo). | `facturacion.certificado.gestionar` | `CertificadoPeticion` | 200 `CertificadoFirmaRespuesta` | 403, 422 |
| POST | `/entidades/{entidad_id}/facturacion/habilitacion/set-pruebas` | Envía al ambiente de habilitación las facturas de prueba y las notas crédito y débito que pida la DIAN, con el TestSetId. | `facturacion.configurar` | `SetPruebasPeticion` | 200 `[EnvioSetRespuesta]` | 403, 422, 500 |
| POST | `/entidades/{entidad_id}/facturacion/habilitacion/set-pruebas/consulta` | Consulta en la DIAN los envíos del set; si todos fueron aceptados, la entidad queda `habilitado`. | `facturacion.configurar` |  | 200 `HabilitacionRespuesta` | 403, 422 |
| POST | `/entidades/{entidad_id}/facturacion/habilitacion/produccion` | Pasa la entidad a producción con el motor propio (exige `habilitado`). | `facturacion.configurar` |  | 200 `HabilitacionRespuesta` | 403, 409 |
| POST | `/entidades/{entidad_id}/facturacion/habilitacion/rangos` | Trae de la DIAN los rangos autorizados al software y registra los que falten como resoluciones de factura (con su clave técnica). Devuelve los nuevos. | `facturacion.configurar` |  | 200 `[RangoSincronizadoRespuesta]` | 403, 422 |
| POST | `/entidades/{entidad_id}/facturacion/compras` | Registra una compra a un proveedor para emitir su documento soporte. Sin contabilidad va sin cuentas, centros de costo ni retenciones; con contabilidad es la misma factura de proveedor de las cuentas por pagar. | `facturacion.facturas.gestionar` `tesoreria.facturas_proveedor.gestionar` | `NuevaFacturaProveedorPeticion` | 201 `FacturaProveedorRespuesta` | 403, 409, 422 |

## nomina

Nómina de cualquier entidad (plan 16): catálogo laboral vigente, empleados (personas de la entidad) con sus contratos laborales versionados, prórroga y terminación, e importación masiva. Los salarios y las cuentas solo se ven con `nomina.salarios.ver`

| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |
|---|---|---|---|---|---|---|
| POST | `/entidades/{entidad_id}/nomina/importaciones` | Sube y valida una planilla de empleados (CSV o XLSX; columnas en la plantilla). No crea nada: devuelve la vista previa con los errores por fila; se aplica con `…/nomina/importaciones/{id}/confirmar`. | `importaciones.ejecutar` `nomina.empleados.gestionar` | `application/octet-stream` | 201 `VistaPreviaEmpleadosRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/nomina/importaciones/plantilla` | Plantilla CSV de empleados (encabezados y una fila de ejemplo, separada por `;`). | _No exige permisos: basta un rol en la entidad_ |  | 200 `text/csv` | 403 |
| POST | `/entidades/{entidad_id}/nomina/importaciones/{importacion_id}/confirmar` | Crea los empleados de una importación validada en una sola transacción (si una fila choca, no se crea ninguno y el mensaje dice la fila). | `importaciones.ejecutar` `nomina.empleados.gestionar` |  | 200 `ImportacionRespuesta` | 403, 409 |
| GET | `/entidades/{entidad_id}/nomina/catalogo` | Catálogo laboral vigente en la fecha y administradoras activas (lo que usan los contratos). | `nomina.ver` |  | 200 `CatalogoLaboralRespuesta` | 403 |
| GET | `/entidades/{entidad_id}/nomina/empleados` | Empleados de la entidad con su estado de hoy. | `nomina.ver` |  | 200 `PaginaRespuesta_EmpleadoResumenRespuesta` | 403 |
| POST | `/entidades/{entidad_id}/nomina/empleados` | Da de alta un empleado con su primer contrato (crea la persona si no está en la entidad). | `nomina.empleados.gestionar` | `NuevoEmpleadoPeticion` | 201 `FichaEmpleadoRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/nomina/empleados/{empleado_id}` | Ficha del empleado con todos sus contratos y versiones (salarios y cuentas solo con `nomina.salarios.ver`). | `nomina.ver` |  | 200 `FichaEmpleadoRespuesta` | 403 |
| GET | `/entidades/{entidad_id}/nomina/contratos/por-vencer` | Contratos a término fijo sin terminar que vencen en los próximos días (los mismos del aviso por correo), del más próximo al más lejano. | `nomina.ver` |  | 200 `[ContratoPorVencerRespuesta]` | 403, 422 |
| POST | `/entidades/{entidad_id}/nomina/empleados/{empleado_id}/contratos` | Agrega un contrato a un empleado (no se cruza en fechas con sus otros contratos). | `nomina.empleados.gestionar` | `NuevoContratoLaboralPeticion` | 201 `ContratoConVersionesRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/nomina/contratos/{contrato_id}/versiones` | Modifica las condiciones del contrato desde una fecha (una versión nueva; las anteriores no cambian). | `nomina.empleados.gestionar` | `ModificacionContratoPeticion` | 201 `VersionContratoRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/nomina/contratos/{contrato_id}/prorroga` | Prorroga un contrato a término fijo. | `nomina.empleados.gestionar` | `ProrrogaContratoPeticion` | 200 `ContratoLaboralRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/nomina/contratos/{contrato_id}/terminacion` | Termina un contrato con su motivo (la liquidación llega en la fase 4). | `nomina.empleados.gestionar` | `TerminacionContratoPeticion` | 200 `ContratoLaboralRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/nomina/configuracion` | Configuración de la nómina (la predeterminada, sin exoneración, si nunca se guardó). | `nomina.ver` |  | 200 `ConfiguracionNominaRespuesta` | 403 |
| PUT | `/entidades/{entidad_id}/nomina/configuracion` | Guarda la configuración de la nómina. | `nomina.configurar` | `ConfiguracionNominaPeticion` | 200 `ConfiguracionNominaRespuesta` | 403 |
| POST | `/entidades/{entidad_id}/nomina/contratos/{contrato_id}/novedades` | Registra una novedad del contrato (dentro de sus fechas y fuera de periodos aprobados). | `nomina.novedades.registrar` | `NuevaNovedadPeticion` | 201 `NovedadNominaRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/nomina/novedades` | Novedades que tocan el rango (con las anuladas marcadas). Las incapacidades y licencias de maternidad o paternidad solo con `personas.datos_sensibles.ver`. | `nomina.ver` |  | 200 `[NovedadNominaRespuesta]` | 403, 422 |
| POST | `/entidades/{entidad_id}/nomina/novedades/{novedad_id}/anulacion` | Anula una novedad (deja de entrar a la liquidación). | `nomina.novedades.registrar` | `AnulacionNovedadPeticion` | 204  | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/nomina/periodos` | Periodos de la entidad, el más reciente primero (a lo sumo 120). | `nomina.ver` |  | 200 `[PeriodoNominaRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/nomina/periodos` | Abre un periodo (el mes completo, o la quincena del 1 al 15 o del 16 al fin de mes). | `nomina.liquidar` | `NuevoPeriodoNominaPeticion` | 201 `PeriodoNominaRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/nomina/periodos/{periodo_id}` | Periodo con sus liquidaciones (datos personales). | `nomina.salarios.ver` |  | 200 `PeriodoLiquidadoRespuesta` | 403 |
| POST | `/entidades/{entidad_id}/nomina/periodos/{periodo_id}/liquidacion` | Liquida (o vuelve a liquidar) todos los contratos del periodo con su periodicidad. | `nomina.liquidar` |  | 200 `PeriodoLiquidadoRespuesta` | 403, 409, 422 |
| POST | `/entidades/{entidad_id}/nomina/periodos/{periodo_id}/aprobacion` | Aprueba la liquidación: otra cuenta distinta de la que liquidó y sin novedades posteriores. | `nomina.aprobar` |  | 200 `PeriodoNominaRespuesta` | 403, 409 |
| POST | `/entidades/{entidad_id}/nomina/periodos/{periodo_id}/pago` | Marca pagado un periodo aprobado. | `nomina.liquidar` | `PagoPeriodoNominaPeticion` | 200 `PeriodoNominaRespuesta` | 403, 409 |
| GET | `/entidades/{entidad_id}/nomina/cesantias` | Cesantías del año por consignar (contratos con salario ordinario vigentes al 31 de diciembre). | `nomina.salarios.ver` |  | 200 `[CesantiasContratoRespuesta]` | 403, 422 |
| GET | `/entidades/{entidad_id}/nomina/liquidaciones/{liquidacion_id}/desprendible` | PDF del desprendible de una liquidación (periodo aprobado o pagado). Las incapacidades y licencias salen como «Ausencia remunerada» y no lleva la cuenta bancaria. | `nomina.salarios.ver` |  | 200 `application/pdf` | 403, 409 |
| POST | `/entidades/{entidad_id}/nomina/periodos/{periodo_id}/desprendibles` | Envía por correo el desprendible de cada empleado del periodo aprobado. | `nomina.liquidar` |  | 200 `EnvioDesprendiblesRespuesta` | 403, 409 |
| GET | `/entidades/{entidad_id}/nomina/empleados/{empleado_id}/certificado-laboral` | Certificado laboral del empleado en PDF (cargo, tipo de contrato y fechas; el salario solo si se pide). | `nomina.ver` `nomina.salarios.ver` |  | 200 `application/pdf` | 403 |
| GET | `/entidades/{entidad_id}/nomina/aportes` | Aportes a la seguridad social y parafiscales del mes, por empleado y por administradora (insumo para diligenciar la PILA en el operador). | `nomina.salarios.ver` |  | 200 `ResumenAportesRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/nomina/ingresos-retenciones` | Ingresos y retenciones del año por empleado, en los grupos del certificado de ingresos y retenciones por rentas de trabajo (insumo del formulario 220). | `nomina.salarios.ver` |  | 200 `[IngresosEmpleadoRespuesta]` | 403, 422 |
| POST | `/entidades/{entidad_id}/nomina/novedades/importaciones` | Sube y valida una planilla de novedades (columnas `codigo` del empleado, `tipo`, `desde`, `hasta`, `cantidad` y `descripcion`). No registra nada: devuelve la vista previa. | `importaciones.ejecutar` `nomina.novedades.registrar` | `application/octet-stream` | 201 `VistaPreviaNovedadesRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/nomina/novedades/importaciones/plantilla` | Plantilla CSV de novedades. | _No exige permisos: basta un rol en la entidad_ |  | 200 `text/csv` | 403 |
| POST | `/entidades/{entidad_id}/nomina/novedades/importaciones/{importacion_id}/confirmar` | Registra las novedades de una importación validada en una sola transacción. | `importaciones.ejecutar` `nomina.novedades.registrar` |  | 200 `ImportacionRespuesta` | 403, 409 |
| POST | `/entidades/{entidad_id}/nomina/contabilizacion` | Contabiliza los periodos aprobados o pagados entre las fechas (la nómina aprobada antes de habilitar la contabilidad). Es idempotente; cada periodo va aparte. | `contabilidad.comprobantes.contabilizar` | `RangoContabilizacionNominaPeticion` | 200 `InformeContabilizacionNominaRespuesta` | 403, 422 |
| POST | `/entidades/{entidad_id}/nomina/contratos/{contrato_id}/simulacion-terminacion` | Simula la terminación del contrato en una fecha y con un motivo: prestaciones, vacaciones pendientes e indemnización, sin guardar nada. | `nomina.salarios.ver` | `SimulacionTerminacionPeticion` | 200 `SimulacionTerminacionRespuesta` | 403, 409, 422 |
| GET | `/entidades/{entidad_id}/nomina/contratos/{contrato_id}/vacaciones` | Saldo de vacaciones del contrato: días causados, disfrutados y pendientes. | `nomina.ver` |  | 200 `SaldoVacacionesRespuesta` | 403, 422 |
| GET | `/entidades/{entidad_id}/nomina/empleados/{empleado_id}/deducciones` | Deducciones de la retención del empleado. | `nomina.salarios.ver` |  | 200 `[DeduccionRetencionRespuesta]` | 403 |
| POST | `/entidades/{entidad_id}/nomina/empleados/{empleado_id}/deducciones` | Registra una deducción de la retención que declaró el empleado (con su soporte). | `nomina.empleados.gestionar` | `NuevaDeduccionPeticion` | 201 `DeduccionRetencionRespuesta` | 403, 422 |
| POST | `/entidades/{entidad_id}/nomina/deducciones/{deduccion_id}/terminacion` | Termina una deducción (desde el día siguiente ya no se aplica). | `nomina.empleados.gestionar` | `TerminacionDeduccionPeticion` | 200 `DeduccionRetencionRespuesta` | 403, 422 |
