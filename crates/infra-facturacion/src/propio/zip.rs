//! ZIP mínimo para enviar documentos a los servicios web de la DIAN (anexo §7.8, §6.5.8).
//!
//! Los servicios reciben el XML firmado dentro de un ZIP en Base64. Basta un ZIP sin compresión
//! (*stored*) con uno o varios archivos; escribirlo son unas decenas de líneas y evita una
//! dependencia (YAGNI). La fecha de los archivos es fija: el mismo documento da el mismo ZIP.

/// CRC-32 (IEEE 802.3) que exige el formato ZIP.
fn crc32(datos: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for b in datos {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            let mascara = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mascara);
        }
    }
    !crc
}

/// Fecha DOS fija (1 de enero de 2000, 00:00): el contenido no depende del reloj.
const FECHA_DOS: u16 = (20 << 9) | (1 << 5) | 1;

fn u16le(v: &mut Vec<u8>, n: u16) {
    v.extend_from_slice(&n.to_le_bytes());
}

fn u32le(v: &mut Vec<u8>, n: u32) {
    v.extend_from_slice(&n.to_le_bytes());
}

/// ZIP sin compresión con los archivos dados (nombre, contenido).
///
/// # Panics
///
/// Nunca en la práctica: un documento electrónico no pasa de 4 GiB ni su nombre de 64 KiB
/// (límites del ZIP sin extensiones Zip64); si pasara, se trunca a esos límites.
pub(crate) fn comprimir(archivos: &[(&str, &[u8])]) -> Vec<u8> {
    let mut salida = Vec::new();
    let mut central = Vec::new();
    for (nombre, contenido) in archivos {
        let desplazamiento = u32::try_from(salida.len()).unwrap_or(u32::MAX);
        let crc = crc32(contenido);
        let largo = u32::try_from(contenido.len()).unwrap_or(u32::MAX);
        let largo_nombre = u16::try_from(nombre.len()).unwrap_or(u16::MAX);

        u32le(&mut salida, 0x0403_4b50);
        u16le(&mut salida, 20); // versión necesaria
        u16le(&mut salida, 0x0800); // nombres en UTF-8
        u16le(&mut salida, 0); // sin compresión
        u16le(&mut salida, 0); // hora
        u16le(&mut salida, FECHA_DOS);
        u32le(&mut salida, crc);
        u32le(&mut salida, largo);
        u32le(&mut salida, largo);
        u16le(&mut salida, largo_nombre);
        u16le(&mut salida, 0);
        salida.extend_from_slice(nombre.as_bytes());
        salida.extend_from_slice(contenido);

        u32le(&mut central, 0x0201_4b50);
        u16le(&mut central, 20); // versión que lo creó
        u16le(&mut central, 20);
        u16le(&mut central, 0x0800);
        u16le(&mut central, 0);
        u16le(&mut central, 0);
        u16le(&mut central, FECHA_DOS);
        u32le(&mut central, crc);
        u32le(&mut central, largo);
        u32le(&mut central, largo);
        u16le(&mut central, largo_nombre);
        u16le(&mut central, 0); // extra
        u16le(&mut central, 0); // comentario
        u16le(&mut central, 0); // disco
        u16le(&mut central, 0); // atributos internos
        u32le(&mut central, 0); // atributos externos
        u32le(&mut central, desplazamiento);
        central.extend_from_slice(nombre.as_bytes());
    }
    let inicio_central = u32::try_from(salida.len()).unwrap_or(u32::MAX);
    let largo_central = u32::try_from(central.len()).unwrap_or(u32::MAX);
    let cantidad = u16::try_from(archivos.len()).unwrap_or(u16::MAX);
    salida.extend_from_slice(&central);
    u32le(&mut salida, 0x0605_4b50);
    u16le(&mut salida, 0);
    u16le(&mut salida, 0);
    u16le(&mut salida, cantidad);
    u16le(&mut salida, cantidad);
    u32le(&mut salida, largo_central);
    u32le(&mut salida, inicio_central);
    u16le(&mut salida, 0);
    salida
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_coincide_con_el_vector_estandar() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn zip_contiene_el_archivo_sin_comprimir_y_el_directorio_central() {
        let zip = comprimir(&[("fv0901713998000270000001.xml", b"<Invoice/>")]);
        assert_eq!(&zip[..4], &[0x50, 0x4b, 0x03, 0x04]);
        let texto = String::from_utf8_lossy(&zip);
        assert_eq!(texto.matches("fv0901713998000270000001.xml").count(), 2);
        assert!(texto.contains("<Invoice/>"));
        let fin = &zip[zip.len() - 22..];
        assert_eq!(&fin[..4], &[0x50, 0x4b, 0x05, 0x06]);
        assert_eq!(u16::from_le_bytes([fin[10], fin[11]]), 1);
    }

    #[test]
    fn el_mismo_contenido_da_el_mismo_zip() {
        let a = comprimir(&[("a.xml", b"x")]);
        let b = comprimir(&[("a.xml", b"x")]);
        assert_eq!(a, b);
    }
}
