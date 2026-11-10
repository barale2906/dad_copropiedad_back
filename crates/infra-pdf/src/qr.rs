//! Imagen PNG de un código QR (puerto [`GeneradorImagenQr`]): el QR temporal de una visita que
//! se envía por WhatsApp. La matriz la calcula `qrcode` y el PNG en escala de grises lo escribe
//! `png`; sin dependencias de imagen pesadas.

use application::{AppError, AppResult, puertos::GeneradorImagenQr};
use qrcode::{Color, EcLevel, QrCode};

/// Píxeles por módulo del QR (legible al reenviarlo y al mostrarlo en el teléfono).
const PIXELES_POR_MODULO: usize = 8;
/// Margen silencioso exigido por el estándar, en módulos.
const MARGEN: usize = 4;

/// Generador de QR en PNG.
#[derive(Debug, Clone, Copy, Default)]
pub struct GeneradorQrPng;

impl GeneradorImagenQr for GeneradorQrPng {
    fn png(&self, texto: &str) -> AppResult<Vec<u8>> {
        // Corrección media: tolera brillo y pantallas rayadas sin crecer demasiado.
        let qr = QrCode::with_error_correction_level(texto.as_bytes(), EcLevel::M)
            .map_err(|_| AppError::Validacion("el texto no cabe en un código QR".into()))?;
        let modulos = qr.width();
        let colores = qr.to_colors();
        let lado = (modulos + 2 * MARGEN) * PIXELES_POR_MODULO;
        let mut pixeles = vec![255u8; lado * lado];
        for (i, c) in colores.iter().enumerate() {
            if *c != Color::Dark {
                continue;
            }
            let (x, y) = (i % modulos + MARGEN, i / modulos + MARGEN);
            for dy in 0..PIXELES_POR_MODULO {
                let fila = (y * PIXELES_POR_MODULO + dy) * lado;
                let inicio = fila + x * PIXELES_POR_MODULO;
                pixeles[inicio..inicio + PIXELES_POR_MODULO].fill(0);
            }
        }
        let mut salida = Vec::new();
        let lado32 = u32::try_from(lado).map_err(AppError::infra)?;
        let mut codificador = png::Encoder::new(&mut salida, lado32, lado32);
        codificador.set_color(png::ColorType::Grayscale);
        codificador.set_depth(png::BitDepth::Eight);
        let mut escritor = codificador.write_header().map_err(AppError::infra)?;
        escritor
            .write_image_data(&pixeles)
            .map_err(AppError::infra)?;
        escritor.finish().map_err(AppError::infra)?;
        Ok(salida)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genera_un_png_valido_del_tamano_esperado() {
        let texto = format!("DQ1.{}.{}", "ab".repeat(51), "cd".repeat(64));

        let png = GeneradorQrPng.png(&texto).expect("png");

        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "firma PNG");
        let decodificador = png::Decoder::new(std::io::Cursor::new(&png));
        let lector = decodificador.read_info().expect("encabezado");
        let info = lector.info();
        assert_eq!(info.width, info.height);
        assert_eq!(info.width as usize % PIXELES_POR_MODULO, 0);
    }

    #[test]
    fn un_texto_enorme_no_cabe() {
        assert!(matches!(
            GeneradorQrPng.png(&"x".repeat(5000)),
            Err(AppError::Validacion(_))
        ));
    }
}
