# ADR 0021 · Motor propio de documentos electrónicos

- **Estado:** aceptada. La modalidad comercial con terceros espera la respuesta de la DIAN a la
  consulta 2026DP000413434.
- **Fecha:** 2026-11-13
- **Plan:** `docs/planes/17-motor-propio-documentos-electronicos.md` (fases 1–4 construidas; 5–7
  diseñadas)

## Contexto
El ADR 0020 dejó el puerto `ProveedorFacturacion` con un proveedor simulado y previó un motor
propio (D3 etapa 2). Se analizó la documentación técnica de la DIAN: anexo técnico de factura
electrónica de venta **v1.9** (Res. 000165 de 2023) y su caja de herramientas (edición 2026). Se
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
