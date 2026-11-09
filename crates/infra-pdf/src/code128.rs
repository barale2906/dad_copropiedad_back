//! Codificación Code 128 (juego B: ASCII imprimible) para los rótulos de la mesa de registro.
//!
//! Se devuelven los anchos (en módulos) de barras y espacios alternados, empezando por una
//! barra: la plantilla Typst los dibuja como rectángulos. Incluye inicio B, dígito de control
//! (módulo 103) y parada.

/// Anchos de los 107 símbolos (barra, espacio, barra, espacio, barra, espacio; la parada tiene
/// una barra final de 2 módulos).
const PATRONES: [&str; 107] = [
    "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212",
    "221213", "221312", "231212", "112232", "122132", "122231", "113222", "123122", "123221",
    "223211", "221132", "221231", "213212", "223112", "312131", "311222", "321122", "321221",
    "312212", "322112", "322211", "212123", "212321", "232121", "111323", "131123", "131321",
    "112313", "132113", "132311", "211313", "231113", "231311", "112133", "112331", "132131",
    "113123", "113321", "133121", "313121", "211331", "231131", "213113", "213311", "213131",
    "311123", "311321", "331121", "312113", "312311", "332111", "314111", "221411", "431111",
    "111224", "111422", "121124", "121421", "141122", "141221", "112214", "112412", "122114",
    "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111", "111242",
    "121142", "121241", "114212", "124112", "124211", "411212", "421112", "421211", "212141",
    "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113", "411311",
    "113141", "114131", "311141", "411131", "211412", "211214", "211232", "2331112",
];

const INICIO_B: usize = 104;
const PARADA: usize = 106;

/// Valores del juego B de cada carácter (ASCII 32–127), o `None` si alguno no es codificable.
fn valores(texto: &str) -> Option<Vec<usize>> {
    texto
        .chars()
        .map(|c| {
            let n = c as u32;
            (32..128).contains(&n).then(|| (n - 32) as usize)
        })
        .collect()
}

/// Símbolos del código: inicio B, datos, control y parada.
pub fn simbolos(texto: &str) -> Option<Vec<usize>> {
    let datos = valores(texto)?;
    if datos.is_empty() {
        return None;
    }
    let control = datos
        .iter()
        .enumerate()
        .fold(INICIO_B, |suma, (i, v)| suma + (i + 1) * v)
        % 103;
    let mut s = Vec::with_capacity(datos.len() + 3);
    s.push(INICIO_B);
    s.extend(datos);
    s.push(control);
    s.push(PARADA);
    Some(s)
}

/// Anchos alternados (barra, espacio, …) de todo el código, sin zonas de silencio.
pub fn modulos(texto: &str) -> Option<Vec<u8>> {
    Some(
        simbolos(texto)?
            .into_iter()
            .flat_map(|s| PATRONES[s].bytes().map(|b| b - b'0'))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cada_patron_mide_once_modulos_y_la_parada_trece() {
        for (i, p) in PATRONES.iter().enumerate() {
            let suma: u32 = p.bytes().map(|b| u32::from(b - b'0')).sum();
            assert_eq!(suma, if i == PARADA { 13 } else { 11 }, "símbolo {i}");
        }
        let mut unicos: Vec<_> = PATRONES.to_vec();
        unicos.sort_unstable();
        unicos.dedup();
        assert_eq!(unicos.len(), 107, "patrones únicos");
    }

    #[test]
    fn digito_de_control() {
        // «PJJ123C»: 104 + 48·1 + 42·2 + 42·3 + 17·4 + 18·5 + 19·6 + 35·7 = 879 ≡ 55 (mód 103).
        let s = simbolos("PJJ123C").expect("codificable");
        assert_eq!(s[0], INICIO_B);
        assert_eq!(s[s.len() - 2], 55);
        assert_eq!(*s.last().expect("parada"), PARADA);
    }

    #[test]
    fn longitud_y_caracteres_no_codificables() {
        // (inicio + 6 datos + control) × 11 + parada 13 = 101 módulos.
        let m = modulos("R-0042").expect("codificable");
        assert_eq!(m.iter().map(|x| u32::from(*x)).sum::<u32>(), 8 * 11 + 13);
        assert!(modulos("Ñ").is_none());
        assert!(modulos("").is_none());
    }
}
