//! Plantilla contable (principio P1 del plan 11): lo que el contador elige para un tipo de
//! empresa y reutiliza. Se guarda como documento (JSONB) y se **copia entera** a la entidad al
//! aplicarla; la entidad sigue editando su copia.
//!
//! Las referencias dentro de la plantilla son por código (de cuenta o de tipo de comprobante),
//! porque los ids nacen al aplicarla en cada entidad.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{
    Cara, CodigoCuenta, MarcoTecnico, Naturaleza, OrigenCentro, OrigenTercero, es_identificador,
    validar_codigo_tipo_comprobante,
};
use crate::DominioError;

/// Cuenta de la plantilla.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CuentaPlantilla {
    pub codigo: CodigoCuenta,
    pub nombre: String,
    pub naturaleza: Naturaleza,
    pub acepta_movimiento: bool,
    #[serde(default)]
    pub exige_tercero: bool,
    #[serde(default)]
    pub exige_centro_costo: bool,
}

/// Tipo de comprobante de la plantilla.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TipoComprobantePlantilla {
    pub codigo: String,
    pub nombre: String,
    /// `false` = solo lo usa el sistema (cierre, apertura, documentos de los módulos).
    pub manual: bool,
}

/// Línea de una regla de la plantilla (sin centro fijo: los centros son de cada entidad).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineaReglaPlantilla {
    pub concepto: String,
    pub cara: Cara,
    pub cuenta: CodigoCuenta,
    pub tercero: OrigenTercero,
    pub centro: OrigenCentro,
}

/// Regla de contabilización de la plantilla.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReglaPlantilla {
    pub evento: String,
    pub tipo_comprobante: String,
    #[serde(default)]
    pub descripcion: Option<String>,
    pub lineas: Vec<LineaReglaPlantilla>,
}

/// Rubro de un estado financiero de la plantilla.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RubroPlantilla {
    /// `situacion_financiera` o `resultados`.
    pub estado: String,
    pub codigo: String,
    pub nombre: String,
    pub orden: u16,
    /// Prefijos de código de cuenta que suma (p. ej. `["11", "12"]`).
    pub prefijos: Vec<String>,
}

/// Contenido de una plantilla contable. Toda sección es opcional: aplicar una plantilla puede
/// traer solo cuentas, solo reglas, etc.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlantillaContable {
    #[serde(default)]
    pub marco_tecnico: Option<MarcoTecnico>,
    #[serde(default)]
    pub cuentas: Vec<CuentaPlantilla>,
    #[serde(default)]
    pub tipos_comprobante: Vec<TipoComprobantePlantilla>,
    #[serde(default)]
    pub reglas: Vec<ReglaPlantilla>,
    #[serde(default)]
    pub rubros: Vec<RubroPlantilla>,
    /// Cuenta de excedente o déficit del ejercicio (clase 3) para el cierre anual.
    #[serde(default)]
    pub cuenta_resultado: Option<CodigoCuenta>,
}

/// Estados financieros que admite un rubro.
pub const ESTADOS_FINANCIEROS: [&str; 2] = ["situacion_financiera", "resultados"];

fn error(texto: impl Into<String>) -> DominioError {
    DominioError::ReglaContable(texto.into())
}

fn nombre_valido(nombre: &str, maximo: usize) -> bool {
    !nombre.trim().is_empty() && nombre.chars().count() <= maximo
}

impl PlantillaContable {
    /// Valida la plantilla por dentro. `existentes` son los códigos de cuenta que ya tiene la
    /// entidad donde se aplicará (sus cuentas pueden servir de padre o de referencia); vacío
    /// para validar la plantilla sola.
    ///
    /// # Errors
    ///
    /// [`DominioError::ReglaContable`] con el primer problema: código repetido, cuenta sin
    /// padre, regla que apunta a una cuenta o tipo que no existe, rubro mal formado…
    pub fn validar(&self, existentes: &BTreeSet<String>) -> Result<(), DominioError> {
        let mut cuentas: BTreeSet<String> = existentes.clone();
        let mut de_movimiento: BTreeSet<&str> = BTreeSet::new();
        for c in &self.cuentas {
            if !nombre_valido(&c.nombre, 200) {
                return Err(error(format!("la cuenta {} no tiene nombre", c.codigo)));
            }
            if c.acepta_movimiento && !c.codigo.admite_movimiento() {
                return Err(error(format!(
                    "la cuenta {} es clase o grupo: no acepta movimientos",
                    c.codigo
                )));
            }
            if !cuentas.insert(c.codigo.as_str().to_owned())
                && !existentes.contains(c.codigo.as_str())
            {
                return Err(error(format!("la cuenta {} está repetida", c.codigo)));
            }
            if c.acepta_movimiento {
                de_movimiento.insert(c.codigo.as_str());
            }
        }
        for c in &self.cuentas {
            if let Some(padre) = c.codigo.padre()
                && !cuentas.contains(padre.as_str())
            {
                return Err(error(format!(
                    "la cuenta {} necesita a su cuenta padre {}",
                    c.codigo, padre
                )));
            }
        }
        let mut tipos = BTreeSet::new();
        for t in &self.tipos_comprobante {
            validar_codigo_tipo_comprobante(&t.codigo)?;
            if !nombre_valido(&t.nombre, 80) || !tipos.insert(t.codigo.as_str()) {
                return Err(error(format!(
                    "el tipo de comprobante {} está repetido o sin nombre",
                    t.codigo
                )));
            }
        }
        let mut eventos = BTreeSet::new();
        for r in &self.reglas {
            if !es_identificador(&r.evento, true, 80) || !eventos.insert(r.evento.as_str()) {
                return Err(error(format!(
                    "el evento `{}` es inválido o repetido",
                    r.evento
                )));
            }
            if !tipos.contains(r.tipo_comprobante.as_str()) {
                return Err(error(format!(
                    "la regla `{}` usa el tipo de comprobante {}, que no está en la plantilla",
                    r.evento, r.tipo_comprobante
                )));
            }
            if r.lineas.len() < 2 {
                return Err(error(format!(
                    "la regla `{}` necesita dos líneas",
                    r.evento
                )));
            }
            for l in &r.lineas {
                if !es_identificador(&l.concepto, false, 60) || l.centro == OrigenCentro::Fijo {
                    return Err(error(format!(
                        "la regla `{}` tiene un concepto inválido o un centro fijo",
                        r.evento
                    )));
                }
                let es_de_la_plantilla = de_movimiento.contains(l.cuenta.as_str());
                if !es_de_la_plantilla && !existentes.contains(l.cuenta.as_str()) {
                    return Err(error(format!(
                        "la regla `{}` usa la cuenta {}, que no es de movimiento en la plantilla",
                        r.evento, l.cuenta
                    )));
                }
            }
        }
        let mut rubros = BTreeSet::new();
        for r in &self.rubros {
            let prefijos_ok = !r.prefijos.is_empty()
                && r.prefijos.iter().all(|p| {
                    !p.is_empty() && p.len() <= 12 && p.chars().all(|c| c.is_ascii_digit())
                });
            if !ESTADOS_FINANCIEROS.contains(&r.estado.as_str())
                || !es_identificador(&r.codigo, false, 60)
                || !nombre_valido(&r.nombre, 160)
                || r.orden == 0
                || !prefijos_ok
                || !rubros.insert((r.estado.as_str(), r.codigo.as_str()))
            {
                return Err(error(format!(
                    "el rubro `{}` está mal formado o repetido",
                    r.codigo
                )));
            }
        }
        if let Some(c) = &self.cuenta_resultado
            && (c.clase() != 3
                || !(de_movimiento.contains(c.as_str()) || existentes.contains(c.as_str())))
        {
            return Err(error(format!(
                "la cuenta de resultado {c} debe ser de patrimonio (clase 3) y de movimiento"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cuenta(codigo: &str, movimiento: bool) -> CuentaPlantilla {
        CuentaPlantilla {
            codigo: CodigoCuenta::new(codigo).expect("código"),
            nombre: format!("Cuenta {codigo}"),
            naturaleza: Naturaleza::Debito,
            acepta_movimiento: movimiento,
            exige_tercero: false,
            exige_centro_costo: false,
        }
    }

    fn plantilla() -> PlantillaContable {
        PlantillaContable {
            marco_tecnico: Some(MarcoTecnico::Grupo3),
            cuentas: vec![
                cuenta("1", false),
                cuenta("11", false),
                cuenta("1105", true),
                cuenta("4", false),
                cuenta("41", false),
                cuenta("4170", true),
                cuenta("3", false),
                cuenta("36", false),
                cuenta("3605", true),
            ],
            tipos_comprobante: vec![TipoComprobantePlantilla {
                codigo: "RC".into(),
                nombre: "Recibo de caja".into(),
                manual: true,
            }],
            reglas: vec![ReglaPlantilla {
                evento: "ventas.contado".into(),
                tipo_comprobante: "RC".into(),
                descripcion: None,
                lineas: vec![
                    LineaReglaPlantilla {
                        concepto: "venta".into(),
                        cara: Cara::Debito,
                        cuenta: CodigoCuenta::new("1105").expect("código"),
                        tercero: OrigenTercero::Ninguno,
                        centro: OrigenCentro::Ninguno,
                    },
                    LineaReglaPlantilla {
                        concepto: "venta".into(),
                        cara: Cara::Credito,
                        cuenta: CodigoCuenta::new("4170").expect("código"),
                        tercero: OrigenTercero::Ninguno,
                        centro: OrigenCentro::Documento,
                    },
                ],
            }],
            rubros: vec![RubroPlantilla {
                estado: "situacion_financiera".into(),
                codigo: "efectivo".into(),
                nombre: "Efectivo y equivalentes".into(),
                orden: 1,
                prefijos: vec!["11".into()],
            }],
            cuenta_resultado: Some(CodigoCuenta::new("3605").expect("código")),
        }
    }

    #[test]
    fn una_plantilla_completa_es_valida_y_viaja_en_json() {
        let p = plantilla();
        p.validar(&BTreeSet::new()).expect("válida");
        let json = serde_json::to_value(&p).expect("json");
        let de_vuelta: PlantillaContable = serde_json::from_value(json).expect("de vuelta");
        assert_eq!(de_vuelta, p);
    }

    #[test]
    fn campos_desconocidos_y_codigos_invalidos_no_se_leen() {
        assert!(serde_json::from_str::<PlantillaContable>(r#"{"otra": 1}"#).is_err());
        assert!(
            serde_json::from_str::<PlantillaContable>(
                r#"{"cuentas":[{"codigo":"11A","nombre":"x","naturaleza":"debito","acepta_movimiento":false}]}"#
            )
            .is_err()
        );
    }

    #[test]
    fn la_cuenta_sin_padre_se_rechaza_salvo_que_la_entidad_lo_tenga() {
        let p = PlantillaContable {
            cuentas: vec![cuenta("110505", true)],
            ..Default::default()
        };
        assert!(p.validar(&BTreeSet::new()).is_err());
        let entidad: BTreeSet<String> = ["1".into(), "11".into(), "1105".into()].into();
        p.validar(&entidad)
            .expect("el padre ya existe en la entidad");
    }

    #[test]
    fn reglas_y_cuenta_de_resultado_deben_apuntar_a_lo_que_existe() {
        let mut sin_tipo = plantilla();
        sin_tipo.tipos_comprobante.clear();
        assert!(sin_tipo.validar(&BTreeSet::new()).is_err());

        let mut a_titulo = plantilla();
        a_titulo.reglas[0].lineas[0].cuenta = CodigoCuenta::new("11").expect("código");
        assert!(
            a_titulo.validar(&BTreeSet::new()).is_err(),
            "11 no es de movimiento"
        );

        let mut resultado_en_activo = plantilla();
        resultado_en_activo.cuenta_resultado = Some(CodigoCuenta::new("1105").expect("código"));
        assert!(resultado_en_activo.validar(&BTreeSet::new()).is_err());
    }

    #[test]
    fn titulos_no_aceptan_movimiento_y_rubros_mal_formados() {
        let titulo = PlantillaContable {
            cuentas: vec![cuenta("1", true)],
            ..Default::default()
        };
        assert!(titulo.validar(&BTreeSet::new()).is_err());
        let mut rubro = plantilla();
        rubro.rubros[0].prefijos = vec!["1A".into()];
        assert!(rubro.validar(&BTreeSet::new()).is_err());
    }
}
