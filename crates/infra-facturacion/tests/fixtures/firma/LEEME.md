# Certificados de prueba (sin validez)

Generados con OpenSSL el 2026-11-13 solo para las pruebas de la firma XAdES del motor propio
(plan 17). La CA es autofirmada y no pertenece a ninguna entidad acreditada por la ONAC: **nada
firmado con ellos tiene validez ante la DIAN**. La clave privada `firmante.key` (PKCS#8) es
pública a propósito; nunca se usa fuera de las pruebas.

- `ca.crt`: CA de pruebas (su clave se descartó).
- `firmante.crt` + `firmante.key`: firmante con `keyUsage` firma digital y no repudio, serial
  `0x6C0B0762626DA0E2` (el mismo número del ejemplo del anexo), vigente 10 años desde su
  generación.
- `sin_no_repudio.crt`: misma clave, sin no repudio (la DIAN la rechaza).
- `firmante_aes.p12` y `firmante_legado.p12`: el firmante con su clave y la CA en PKCS#12, contraseña
  `prueba-123`. El primero en el formato moderno de OpenSSL 3 (PBES2, PBKDF2-SHA256, AES-256-CBC,
  MAC SHA-256); el segundo en el heredado (`-legacy`: 3DES para la llave, RC2-40 para los
  certificados, MAC SHA-1). Generados con `openssl pkcs12 -export [-legacy] -inkey firmante.key
  -in firmante.crt -certfile ca.crt -passout pass:prueba-123 -name firmante`.
