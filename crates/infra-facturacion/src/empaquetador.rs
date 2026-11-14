//! Empaquetador ZIP (puerto [`Empaquetador`]): reutiliza el ZIP sin compresión del motor propio
//! (el mismo con el que se envían los documentos a la DIAN) para el paquete de XML y PDF que la
//! entidad entrega a su contador.

use application::{AppError, AppResult, puertos::archivos::Empaquetador};

use crate::propio::zip::comprimir;

/// Límite del ZIP sin extensiones Zip64.
const MAXIMO_BYTES: usize = u32::MAX as usize;

/// ZIP sin compresión, determinista (fecha fija dentro del ZIP).
#[derive(Debug, Clone, Copy, Default)]
pub struct EmpaquetadorZip;

impl Empaquetador for EmpaquetadorZip {
    fn empaquetar(&self, archivos: &[(String, Vec<u8>)]) -> AppResult<Vec<u8>> {
        let total: usize = archivos.iter().map(|(n, c)| n.len() + c.len() + 128).sum();
        if archivos.len() > usize::from(u16::MAX) || total >= MAXIMO_BYTES {
            return Err(AppError::Validacion(
                "el paquete es demasiado grande: acote el periodo".into(),
            ));
        }
        let entradas: Vec<(&str, &[u8])> = archivos
            .iter()
            .map(|(n, c)| (n.as_str(), c.as_slice()))
            .collect();
        Ok(comprimir(&entradas))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_paquete_trae_cada_archivo_con_su_nombre() {
        let zip = EmpaquetadorZip
            .empaquetar(&[
                ("FE-1.xml".into(), b"<Invoice/>".to_vec()),
                ("FE-1.pdf".into(), b"%PDF".to_vec()),
            ])
            .expect("zip");

        assert_eq!(&zip[..4], b"PK\x03\x04");
        let texto = String::from_utf8_lossy(&zip);
        assert!(texto.contains("FE-1.xml") && texto.contains("FE-1.pdf"));
        assert!(texto.contains("<Invoice/>"));
    }

    #[test]
    fn el_mismo_contenido_da_el_mismo_paquete() {
        let a = vec![("a.xml".to_owned(), b"x".to_vec())];
        assert_eq!(
            EmpaquetadorZip.empaquetar(&a).expect("zip"),
            EmpaquetadorZip.empaquetar(&a).expect("zip")
        );
    }
}
