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
