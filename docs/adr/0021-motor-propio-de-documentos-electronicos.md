# ADR 0021 · Motor propio de documentos electrónicos

- **Estado:** aceptada. La DIAN respondió la consulta 2026DP000413434 (oficio 100153157-18879
  del 2026-11-26, ver la adenda final); el escenario SaaS con terceros sigue sin confirmar.
- **Fecha:** 2026-11-13
- **Plan:** `docs/planes/17-motor-propio-documentos-electronicos.md` (fases 1–4 construidas; 5–7
  diseñadas)

## Contexto
El ADR 0020 dejó el puerto `ProveedorFacturacion` con un proveedor simulado y previó un motor
propio (D3 etapa 2). Se analizó la documentación técnica de la DIAN: anexo técnico de factura
electrónica de venta **v1.9** (Res. 000165 de 2023, hoy compilada en el título 5 de la
Res. 000227 de 2025) y su caja de herramientas (edición 2026). Se
revisaron también los requisitos para ser proveedor tecnológico (art. 55 de la Res. 000165:
patrimonio de al menos 20.000 UVT, PP&E de al menos 10.000 UVT, ISO 27001, visita de la DIAN,
personal acreditado y renovación cada 5 años). El art. 28 num. 2.1 permite que el facturador se
habilite con un "desarrollo informático propio o adquirido" registrando al fabricante del
software, sin que este sea proveedor tecnológico.

## Decisión
- **GISLA SAS (NIT 901713998-3) no se habilita como proveedor tecnológico.** Es el fabricante
  del software. GISLA se habilita primero como facturadora en modo software propio (piloto). Cada
  entidad cliente se habilitaría con su propio certificado y su propio set de pruebas. Se
  consultó formalmente a la DIAN si eso vale operando como SaaS (escenario a) o solo en la
  infraestructura del cliente (escenario b). Si ninguno es posible, un proveedor tecnológico entra
  como otro adaptador del mismo puerto.
- **Motor en `infra-facturacion::propio`, en Rust puro y sin dependencias nuevas:**
  - `control`: CUFE (clave técnica), CUDE (PIN del software), código de seguridad del software y
    QR. Los montos se truncan a dos decimales desde `Decimal`. Se prueba con los ejemplos
    oficiales del anexo.
  - `ubl`: factura, nota crédito y nota débito en UBL 2.1 con `DianExtensions`. Usa un modelo
    propio y completo (`DocumentoUbl`), que el adaptador llenará desde el `DocumentoAEmitir`
    del puerto y la configuración del facturador.
  - `firma`: XAdES-EPES (XMLDSig *enveloped*, C14N 1.0, RSA-SHA256), cadena completa en
    `X509Data` y en `SigningCertificate`, política de firma v2 con su hash. Rechaza certificados
    vencidos o sin no repudio.
  - `servicio`: SOAP 1.2 + WS-Addressing + WS-Security (`BinarySecurityToken`, `Timestamp`,
    firma de `wsa:To` con C14N exclusiva) sobre reqwest/rustls. Operaciones `SendBillSync`,
    `SendBillAsync`, `SendTestSetAsync`, `GetStatus`, `GetStatusZip`, `GetNumberingRange`,
    `GetXmlByDocumentKey` y `SendEventUpdateStatus`. La URL llega por configuración.
- **Canonicalización, X.509 y ZIP propios y mínimos.** No hay un C14N/XAdES maduro en Rust puro,
  y `libxmlsec1` arrastra OpenSSL (AGENTS.md §4). El motor arma los documentos con su propio
  árbol XML y los serializa **siempre** en forma canónica: lo firmado es exactamente lo enviado.
  El lector X.509 solo extrae emisor (RFC 2253), serial, vigencia y `keyUsage`. El ZIP es
  *stored*.
- **La clave técnica, el PIN y la clave privada nunca van al XML, a los logs ni a `Debug`**
  (`Software`, `Resolucion`, `RangoNumeracion` y `CredencialFirma` los ocultan).

## Consecuencias
- Verificado de forma independiente el 2026-11-13: los XML generados cumplen los **XSD de la caja
  de herramientas**, y las firmas pasan un verificador XAdES externo (signxml/lxml). La firma
  WS-Security pasa con la C14N exclusiva de lxml. El Schematron de la DIAN (XSLT 3.0) queda como
  chequeo manual con Saxon. La validación definitiva la da el ambiente de habilitación.
- Pruebas determinísticas contra un simulador local del servicio web; nunca contra la DIAN en
  las pruebas automáticas. Los certificados de prueba están en
  `crates/infra-facturacion/tests/fixtures/firma` (CA autofirmada, sin validez).
- **Pendiente (fase 5):** el adaptador `ProveedorFacturacion` del motor, la configuración por
  entidad (SoftwareID, PIN, TestSetId, certificado PKCS#12 cifrado con clave maestra), el
  asistente de habilitación, el consecutivo anual de archivos y la sincronización de rangos.
  También falta leer PKCS#12 (crate en Rust puro por evaluar). **Fases 6–7:** AttachedDocument,
  representación gráfica con QR, contingencia tipo 04 y eventos RADIAN propios.
- `GetAcquirer`, `GetExchangeEmails`, `GetStatusEvent` y `GetReferenceNotes` se agregan cuando
  una fase los use.

## Adenda (2026-11-14): habilitación por entidad (plan 18 parte B, plan 17 fase 5)
- **Lector PKCS#12 propio** (`propio::pkcs12`): estructura y derivación de claves de RFC 7292 sobre
  el lector DER propio; cifrados de RustCrypto (`aes`, `cbc`, `des`, `rc2`) y PBKDF2 de `ring`.
  Lee el formato moderno (PBES2, AES-256-CBC, MAC SHA-256) y el heredado (3DES, RC2-40, MAC SHA-1);
  el firmante es el certificado cuya clave pública corresponde a la llave. Se descartó
  `p12-keystore` (unas 105 dependencias) y `pkcs12` de RustCrypto (no descifra).
- **Secretos por entidad cifrados** (`CofreAesGcm`, AES-256-GCM con `FACTURACION_CLAVE_MAESTRA`;
  el contexto `facturacion.<uso>:<tenant>` va como AAD): PIN, `.p12` y su contraseña en la BD,
  nunca en claro. El `.p12` cifrado va en `certificados_firma.p12_cifrado` (no en el almacén de
  archivos: su tipo no es verificable por contenido y conviene tenerlo junto a su contraseña).
- **Habilitación**: `configuraciones_facturacion` con `EstadoHabilitacion` (registrado → en pruebas
  → habilitado → producción). El set de pruebas se arma con facturas al consumidor final en el
  rango `SETP` 990000000–995000000 con la clave técnica pública de habilitación y se envía con
  `SendTestSetAsync`; se consulta con `GetStatusZip` y, si todo fue aceptado, la entidad queda
  habilitada. En producción se sincronizan los rangos con `GetNumberingRange`.
- **Transmisión**: `ProveedorDian` envía por el motor propio el documento que trae credenciales
  (canal `propio` del perfil confirmado y entidad en producción) y por el simulado los demás. En
  software propio el NIT del proveedor del software es el del facturador.
- **Pendiente**: notas del set de pruebas (hoy solo facturas), documento soporte y eventos RADIAN
  propios (fase 7), Schematron con Saxon, y la validación real contra el ambiente de habilitación
  con el `.p12` de GISLA. (La contingencia tipo 04 y el AttachedDocument se resolvieron en la
  adenda siguiente.)

## Adenda (2026-11-14): expedición completa y contingencia (plan 17 fase 6)
- **AttachedDocument** (`propio::adjunto`, anexo 1.9 §7.10): contenedor firmado con el mismo
  certificado que lleva el documento y, si la DIAN respondió, su `ApplicationResponse`
  (`ValidationResultCode` 02); en contingencia va sin respuesta (04). Los XML internos van
  escapados en `cbc:Description` (equivale al `CDATA`; la forma canónica no cambia). Se guarda en
  el almacén (`documentos_electronicos.adjunto_archivo_id`).
- **Entrega al adquirente** por el outbox: correo con un ZIP (AttachedDocument, o XML + PDF con el
  simulado) al validarse y, en contingencia, la primera vez. `CorreoSaliente` admite adjuntos
  (`AdjuntoCorreo`, contenido en hexadecimal dentro del JSON del outbox; `infra-correo` arma
  `multipart/mixed`). El correo de recepción es el `correo_facturacion` del tercero; leerlo de
  `GetExchangeEmails` queda para después.
- **Representación gráfica**: QR (2,4 cm, URL de consulta de la DIAN con el CUFE) en el
  encabezado de cada página y el pie con el fabricante y el software (`FACTURACION_SOFTWARE_NOMBRE`,
  `FACTURACION_FABRICANTE_NOMBRE`, `FACTURACION_FABRICANTE_NIT`).
- **Contingencia DIAN (tipo 04)**: `MotorPropio` reintenta 3 veces con 5 s de espera; si la DIAN
  sigue sin responder, firma la factura con `InvoiceTypeCode` 04 (mismo prefijo y número) y la
  devuelve en `contingencia` con su AttachedDocument. Las notas no tienen contingencia (error de
  infraestructura). El worker `worker_facturacion` del gateway reintenta cada 30 minutos los
  documentos en contingencia de todas las entidades (`documentos_en_contingencia`, SECURITY
  DEFINER, solo ids) y avisa en los logs los que pasan de 48 horas.
- **Pendiente**: contingencia del facturador (tipo 03, transcripción del talonario) y el correo de
  recepción desde `GetExchangeEmails`.

## Adenda (2026-11-14): cierre del motor propio (plan 17 fase 7)
- **Eventos RADIAN propios** (`propio::evento`, anexo 1.9 §6.5 y §11.5): `ApplicationResponse`
  firmado por quien genera el evento, con CUDE (SHA-384 con el PIN; verificado con el ejemplo
  oficial), enviado con `SendEventUpdateStatus`. Acuse (030) y recibo del bien (032) identifican a
  la persona que recibió; el reclamo (031) lleva su concepto (lista de la DIAN); la aceptación
  tácita (034) la emite el facturador sobre su factura a crédito, dirigida a la DIAN y con la
  constancia bajo juramento. Número del evento `EV` + año + consecutivo de la entidad; XML en el
  almacén. Sin el canal `propio`, el evento lo registra el proveedor configurado.
- **Documento soporte y nota de ajuste** (anexo técnico del documento soporte v1.1, Res. 000167
  de 2021): `Invoice` tipo 05 y `CreditNote` tipo 95 con CUDS (verificado con el ejemplo
  oficial), el vendedor no obligado como proveedor (con su ubicación) y la entidad como
  adquirente, `InvoicePeriod` por operación y la identificación estándar del ítem (`999`).
  Archivos `ds`/`nas`. Sin contingencia (suplemento C del anexo) y sin contenedor. La nota de
  ajuste anula el documento soporte y la compra en la misma transacción.
- **Numeración de las notas**: las notas crédito, débito y de ajuste no usan resolución, pero
  cada una lleva número propio: prefijo del tipo y del año (`NC26`, `ND26`, `NA26`) y
  consecutivo anual de la entidad (series `nota_*` en `consecutivos`). Antes salían sin número.
- **Contingencia del facturador (tipo 03)**: resolución de numeración `contingencia` (sin clave
  técnica); la factura de papel se transcribe con el número del talonario, `InvoiceTypeCode` 03,
  CUDE con el PIN y `AdditionalDocumentReference` a la factura de papel (`DocumentTypeCode`
  propio `FTC`). **Por confirmar en las pruebas de habilitación**: el anexo exige un UUID en esa
  referencia y no dice cuál para una factura de papel; se usa el CUDE del propio documento.
- **Correos de recepción** (`GetExchangeEmails`, §7.16): la DIAN entrega el correo de todos los
  facturadores; el worker del gateway lo sincroniza a diario en `correos_recepcion_dian` con las
  credenciales de la entidad de la plataforma, y la entrega al adquirente lo usa antes que el
  correo de facturación del tercero.
- **Set de pruebas** con notas crédito y débito (cada una referida a una factura del mismo
  envío). La nota débito usa ahora el concepto 3 («cambio del valor»), no el 2.
- Todos los tipos (factura, tipo 03, notas, documento soporte, nota de ajuste y evento) validan
  contra los XSD de la caja de herramientas 1.9.
- **Nota débito** (decisión del usuario del 2026-11-14): es un documento por cobrar más del
  cliente con la misma forma que la factura, así que se guarda en `facturas_venta` con la factura
  que ajusta (`nota_debito_de`) y su concepto de la DIAN. Entra a la cartera por edades, se abona
  con recibos de caja o se marca pagada a mano y, con contabilidad, se contabiliza con la regla de
  la factura de venta; se transmite referida a la factura electrónica original, todo en una
  transacción (`documentos::emitir_nota_debito`). No admite su propia factura electrónica ni
  nota crédito (anular una nota débito queda para después).
- **Pendiente**: Schematron con Saxon y el piloto con el `.p12` de GISLA.

## Adenda (2026-11-27): respuesta de la DIAN a la consulta 2026DP000413434
Oficio 100153157-18879 del 2026-11-26 de la Subdirección de Factura Electrónica y Soluciones
Operativas. Es una **orientación general, no un concepto**: solo la Subdirección de Normativa y
Doctrina interpreta la norma con efecto vinculante.

- **Norma vigente:** la Res. 000165 de 2023 está compilada en el **título 5 de la Res. 000227 de
  2025** (resolución única en materia tributaria, aduanera y cambiaria). El anexo técnico **1.9**
  sigue vigente. El Concepto 013246 de 2025 (subnumeral 3.1.3.1 del Concepto Unificado 0106 de
  2022) define software propio (lo desarrolla el facturador) y adquirido (lo compra a un tercero).
- **Software en la nube del fabricante:** alojar el aplicativo en infraestructura del fabricante,
  administrar el servidor, mantener, actualizar y dar soporte **no son, por sí solos**, un
  criterio que determine la modalidad.
- **¿GISLA debe ser proveedor tecnológico en el escenario SaaS (a)?** Sin respuesta directa: se
  decide por las **actividades que efectivamente se desarrollen** dentro del sistema de
  facturación (art. 616-4 del Estatuto Tributario, art. 1.6.1.4.2.4 del Decreto 1625 de 2016),
  no por el nombre del servicio, el modelo de negocio ni la infraestructura. Como el SaaS genera,
  transmite y conserva desde servidores de GISLA, el riesgo sigue abierto.
- **Certificado:** es de cada facturador y se usa en todo el proceso, firma y WS-Security. Ya se
  cumple (`CredencialFirma` por entidad).
- **Registro e identificadores:** cada facturador se habilita y registra el software por su
  cuenta y recibe sus propios identificadores (SoftwareID). Ya se cumple
  (`configuraciones_facturacion` por entidad; `ProviderID` = NIT del facturador).

**Consecuencias:**
- El motor no cambia. Las citas a la Res. 000165 se mantienen y se aclara que está compilada en
  la Res. 000227.
- El piloto con GISLA sigue sin bloqueos.
- **Para clientes externos**, hasta tener un concepto favorable de Normativa y Doctrina o de un
  asesor tributario, se ofrece: (b) despliegue en la infraestructura del cliente
  (`docker-compose.prod.yml`) o un proveedor tecnológico como otro adaptador de
  `ProveedorFacturacion`. Si se ofrece SaaS, la operación queda en manos del facturador: contrato
  de licencia de uso (no de servicio de facturación), el cliente compra su certificado, registra
  el software y carga sus credenciales, y puede exportar en cualquier momento todos sus XML y
  AttachedDocument.
- **Pendiente:** decidir si se eleva a Normativa y Doctrina una consulta con una sola pregunta:
  si alojar y operar en servidores del fabricante la generación, transmisión y conservación de
  los documentos de un facturador con su propio certificado y SoftwareID es prestar el servicio
  del art. 616-4.
