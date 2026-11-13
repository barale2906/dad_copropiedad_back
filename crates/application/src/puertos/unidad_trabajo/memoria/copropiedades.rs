//! Dobles en memoria de los repositorios de copropiedades y bienes privados.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use domain::{
    BienPrivadoId, Coeficiente, CuadroCoeficientesId, CuentaId, Documento, EstadoCopropiedad,
    EstadoCuadro, FormaCopropiedad, Horario, PersonaId, PlantillaNomenclatura, PuntoAtencionId,
    TipoCopropiedad, TitularidadId, UsoExclusivoId,
};

use super::TransaccionEnMemoria;
use crate::{
    AppError, AppResult, Pagina, SolicitudPagina,
    puertos::{
        RepositorioBienesPrivados, RepositorioCoeficientes, RepositorioCopropiedad,
        RepositorioPersonas, RepositorioTitulares,
        copropiedades::{
            Agrupacion, BienPrivado, CargoVigente, CuadroCoeficientes, FichaCopropiedad,
            FiltroBienes, Persona, PuntoAtencion, Titularidad, UsoExclusivo,
        },
    },
};

impl RepositorioCopropiedad for TransaccionEnMemoria {
    async fn ficha(&mut self) -> AppResult<Option<FichaCopropiedad>> {
        Ok(Some(FichaCopropiedad {
            nombre: "PH en memoria".into(),
            razon_social: None,
            nit: None,
            tipo: TipoCopropiedad::Residencial,
            forma: FormaCopropiedad::Conjunto,
            direccion: None,
            municipio_dane: None,
            departamento: None,
            barrio: None,
            latitud: None,
            longitud: None,
            estrato: None,
            telefono: None,
            correo: None,
            sitio_web: None,
            escritura_numero: None,
            notaria: None,
            escritura_fecha: None,
            matricula_matriz: None,
            reglamento_archivo_id: None,
            certificado_archivo_id: None,
            zona_horaria: "America/Bogota".into(),
            estado: EstadoCopropiedad::EnConfiguracion,
            plantilla_codigo: PlantillaNomenclatura::default(),
        }))
    }

    async fn guardar_ficha(&mut self, _ficha: &FichaCopropiedad) -> AppResult<()> {
        Ok(())
    }

    async fn cargos_vigentes(&mut self) -> AppResult<Vec<CargoVigente>> {
        Ok(self.datos.asambleas.cargos_vigentes.clone())
    }

    async fn bienes_para_consejo(&mut self) -> AppResult<u64> {
        Ok(self
            .datos
            .bienes
            .values()
            .filter(|b| b.activo && b.tipo.cuenta_para_consejo())
            .count() as u64)
    }

    async fn puntos_atencion(&mut self) -> AppResult<Vec<PuntoAtencion>> {
        Ok(Vec::new())
    }

    async fn insertar_punto_atencion(&mut self, _punto: &PuntoAtencion) -> AppResult<()> {
        Ok(())
    }

    async fn reemplazar_horario(
        &mut self,
        _punto: PuntoAtencionId,
        _horario: &Horario,
    ) -> AppResult<bool> {
        Ok(false)
    }
}

impl RepositorioBienesPrivados for TransaccionEnMemoria {
    async fn agrupaciones(&mut self) -> AppResult<Vec<Agrupacion>> {
        Ok(self.datos.agrupaciones.clone())
    }

    async fn insertar_agrupacion(&mut self, a: &Agrupacion) -> AppResult<()> {
        if self
            .datos
            .agrupaciones
            .iter()
            .any(|x| x.padre_id == a.padre_id && x.codigo == a.codigo)
        {
            return Err(AppError::Conflicto("código de agrupación repetido".into()));
        }
        self.datos.agrupaciones.push(a.clone());
        Ok(())
    }

    async fn bien(&mut self, id: BienPrivadoId) -> AppResult<Option<BienPrivado>> {
        Ok(self.datos.bienes.get(&id).cloned())
    }

    async fn todos_los_bienes(&mut self) -> AppResult<Vec<BienPrivado>> {
        let mut v: Vec<_> = self.datos.bienes.values().cloned().collect();
        v.sort_by(|a, b| a.codigo.as_str().cmp(b.codigo.as_str()));
        Ok(v)
    }

    async fn listar_bienes(
        &mut self,
        filtro: &FiltroBienes,
        pagina: SolicitudPagina,
    ) -> AppResult<Pagina<BienPrivado>> {
        let filas: Vec<_> = self
            .datos
            .bienes
            .values()
            .filter(|b| filtro.incluir_inactivos || b.activo)
            .filter(|b| filtro.tipo.is_none_or(|t| b.tipo == t))
            .filter(|b| {
                filtro
                    .agrupacion_id
                    .is_none_or(|g| b.agrupacion_id == Some(g))
            })
            .filter(|b| pagina.despues().is_none_or(|d| b.id.as_uuid() > d))
            .take(usize::try_from(pagina.filas_a_consultar()).unwrap_or(usize::MAX))
            .cloned()
            .collect();
        Ok(Pagina::desde_filas(filas, pagina, |b| b.id.as_uuid()))
    }

    async fn insertar_bien(&mut self, bien: &BienPrivado) -> AppResult<()> {
        if self.datos.bienes.values().any(|b| b.codigo == bien.codigo) {
            return Err(AppError::Conflicto("código repetido".into()));
        }
        self.datos.bienes.insert(bien.id, bien.clone());
        Ok(())
    }

    async fn actualizar_bien(&mut self, bien: &BienPrivado) -> AppResult<()> {
        if self
            .datos
            .bienes
            .values()
            .any(|b| b.codigo == bien.codigo && b.id != bien.id)
        {
            return Err(AppError::Conflicto("código repetido".into()));
        }
        let coeficiente = self.datos.bienes.get(&bien.id).and_then(|b| b.coeficiente);
        self.datos.bienes.insert(
            bien.id,
            BienPrivado {
                coeficiente,
                ..bien.clone()
            },
        );
        Ok(())
    }

    async fn anexos(&mut self, principal: BienPrivadoId) -> AppResult<Vec<BienPrivado>> {
        Ok(self
            .datos
            .bienes
            .values()
            .filter(|b| b.bien_principal_id == Some(principal))
            .cloned()
            .collect())
    }

    async fn usos_exclusivos(&mut self, bien: BienPrivadoId) -> AppResult<Vec<UsoExclusivo>> {
        Ok(self
            .datos
            .usos_exclusivos
            .iter()
            .filter(|u| u.bien_privado_id == bien)
            .cloned()
            .collect())
    }

    async fn insertar_uso_exclusivo(&mut self, uso: &UsoExclusivo) -> AppResult<()> {
        if self
            .datos
            .usos_exclusivos
            .iter()
            .any(|u| u.vigente_hasta.is_none() && u.tipo == uso.tipo && u.codigo == uso.codigo)
        {
            return Err(AppError::Conflicto("bien común ya asignado".into()));
        }
        self.datos.usos_exclusivos.push(uso.clone());
        Ok(())
    }

    async fn terminar_uso_exclusivo(
        &mut self,
        bien: BienPrivadoId,
        id: UsoExclusivoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        let Some(u) = self
            .datos
            .usos_exclusivos
            .iter_mut()
            .find(|u| u.id == id && u.bien_privado_id == bien)
        else {
            return Ok(false);
        };
        if u.vigente_hasta.is_none() {
            u.vigente_hasta = Some(ahora.max(u.vigente_desde));
        }
        Ok(true)
    }
}

impl RepositorioCoeficientes for TransaccionEnMemoria {
    async fn cuadros(&mut self) -> AppResult<Vec<CuadroCoeficientes>> {
        let mut v = self.datos.cuadros.clone();
        v.sort_by_key(|c| std::cmp::Reverse(c.version));
        Ok(v)
    }

    async fn cuadro(&mut self, id: CuadroCoeficientesId) -> AppResult<Option<CuadroCoeficientes>> {
        Ok(self.datos.cuadros.iter().find(|c| c.id == id).cloned())
    }

    async fn cuadro_vigente(
        &mut self,
        modulo: Option<domain::ModuloContribucionId>,
    ) -> AppResult<Option<CuadroCoeficientes>> {
        Ok(self
            .datos
            .cuadros
            .iter()
            .find(|c| c.estado == EstadoCuadro::Vigente && c.modulo_id == modulo)
            .cloned())
    }

    async fn ultima_version(
        &mut self,
        modulo: Option<domain::ModuloContribucionId>,
    ) -> AppResult<u32> {
        Ok(self
            .datos
            .cuadros
            .iter()
            .filter(|c| c.modulo_id == modulo)
            .map(|c| c.version)
            .max()
            .unwrap_or(0))
    }

    async fn modulo_contribucion_activo(
        &mut self,
        _modulo: domain::ModuloContribucionId,
    ) -> AppResult<bool> {
        Ok(false)
    }

    async fn insertar_cuadro(&mut self, cuadro: &CuadroCoeficientes) -> AppResult<()> {
        self.datos.cuadros.push(cuadro.clone());
        Ok(())
    }

    async fn coeficientes(
        &mut self,
        cuadro: CuadroCoeficientesId,
    ) -> AppResult<BTreeMap<BienPrivadoId, Coeficiente>> {
        Ok(self
            .datos
            .coeficientes
            .get(&cuadro)
            .cloned()
            .unwrap_or_default())
    }

    async fn reemplazar_coeficientes(
        &mut self,
        cuadro: CuadroCoeficientesId,
        coeficientes: &BTreeMap<BienPrivadoId, Coeficiente>,
    ) -> AppResult<()> {
        self.datos.coeficientes.insert(cuadro, coeficientes.clone());
        Ok(())
    }

    async fn activar(
        &mut self,
        cuadro: CuadroCoeficientesId,
        anterior: Option<CuadroCoeficientesId>,
        ahora: DateTime<Utc>,
        aprobado_por: Option<CuentaId>,
    ) -> AppResult<()> {
        for c in &mut self.datos.cuadros {
            if Some(c.id) == anterior {
                c.estado = EstadoCuadro::Historico;
            }
            if c.id == cuadro {
                c.estado = EstadoCuadro::Vigente;
                c.vigente_desde = Some(ahora);
                c.aprobado_por = aprobado_por;
            }
        }
        let filas = self
            .datos
            .coeficientes
            .get(&cuadro)
            .cloned()
            .unwrap_or_default();
        for b in self.datos.bienes.values_mut() {
            b.coeficiente = filas.get(&b.id).copied();
        }
        Ok(())
    }

    async fn reuniones_instaladas(&mut self) -> AppResult<u64> {
        Ok(self.datos.reuniones_instaladas)
    }
}

impl RepositorioPersonas for TransaccionEnMemoria {
    async fn persona(&mut self, id: PersonaId) -> AppResult<Option<Persona>> {
        Ok(self.datos.personas.iter().find(|p| p.id == id).cloned())
    }

    async fn persona_por_documento(&mut self, documento: &Documento) -> AppResult<Option<Persona>> {
        Ok(self
            .datos
            .personas
            .iter()
            .find(|p| p.documento.as_ref() == Some(documento))
            .cloned())
    }

    async fn insertar_persona(&mut self, persona: &Persona) -> AppResult<()> {
        self.datos.personas.push(persona.clone());
        Ok(())
    }

    async fn vincular_cuenta(&mut self, persona: PersonaId, cuenta: CuentaId) -> AppResult<()> {
        if let Some(p) = self.datos.personas.iter_mut().find(|p| p.id == persona) {
            p.cuenta_id = Some(cuenta);
        }
        Ok(())
    }

    async fn actualizar_persona(&mut self, persona: &Persona) -> AppResult<()> {
        if let Some(p) = self.datos.personas.iter_mut().find(|p| p.id == persona.id) {
            *p = persona.clone();
        }
        Ok(())
    }

    async fn listar_personas(
        &mut self,
        texto: Option<&str>,
        pagina: SolicitudPagina,
    ) -> AppResult<Pagina<Persona>> {
        let mut filas: Vec<_> = self
            .datos
            .personas
            .iter()
            .filter(|p| texto.is_none_or(|t| p.nombre.to_lowercase().contains(&t.to_lowercase())))
            .filter(|p| pagina.despues().is_none_or(|d| p.id.as_uuid() > d))
            .cloned()
            .collect();
        filas.sort_by_key(|p| p.id);
        filas.truncate(usize::try_from(pagina.filas_a_consultar()).unwrap_or(usize::MAX));
        Ok(Pagina::desde_filas(filas, pagina, |p| p.id.as_uuid()))
    }
}

impl RepositorioTitulares for TransaccionEnMemoria {
    async fn titulares_vigentes(&mut self, bien: BienPrivadoId) -> AppResult<Vec<Titularidad>> {
        Ok(self
            .datos
            .titularidades
            .iter()
            .filter(|t| t.bien_privado_id == bien && t.vigente_hasta.is_none())
            .cloned()
            .collect())
    }

    async fn historial_titulares(&mut self, bien: BienPrivadoId) -> AppResult<Vec<Titularidad>> {
        Ok(self
            .datos
            .titularidades
            .iter()
            .rev()
            .filter(|t| t.bien_privado_id == bien)
            .cloned()
            .collect())
    }

    async fn insertar_titularidad(&mut self, t: &Titularidad) -> AppResult<()> {
        self.datos.titularidades.push(t.clone());
        Ok(())
    }

    async fn terminar_titulares(
        &mut self,
        bien: BienPrivadoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<TitularidadId>> {
        let mut ids = Vec::new();
        for t in &mut self.datos.titularidades {
            if t.bien_privado_id == bien && t.vigente_hasta.is_none() {
                t.vigente_hasta = Some(ahora.max(t.vigente_desde));
                ids.push(t.id);
            }
        }
        Ok(ids)
    }

    async fn designar_representante(
        &mut self,
        bien: BienPrivadoId,
        persona: PersonaId,
    ) -> AppResult<()> {
        for t in &mut self.datos.titularidades {
            if t.bien_privado_id == bien && t.vigente_hasta.is_none() {
                t.es_representante = t.persona_id == persona;
            }
        }
        Ok(())
    }

    async fn bienes_de_cuenta(&mut self, cuenta: CuentaId) -> AppResult<Vec<BienPrivadoId>> {
        let personas: Vec<PersonaId> = self
            .datos
            .personas
            .iter()
            .filter(|p| p.cuenta_id == Some(cuenta))
            .map(|p| p.id)
            .collect();
        let mut ids: Vec<_> = self
            .datos
            .titularidades
            .iter()
            .filter(|t| t.vigente_hasta.is_none() && personas.contains(&t.persona_id))
            .map(|t| t.bien_privado_id)
            .collect();
        ids.dedup();
        Ok(ids)
    }
}

/// El doble en memoria no lleva cartera: el chat responde que no hay datos.
impl crate::puertos::cartera::ResumenCartera for TransaccionEnMemoria {
    async fn resumen_bien(
        &mut self,
        _bien: BienPrivadoId,
        _hoy: chrono::NaiveDate,
    ) -> AppResult<Option<crate::puertos::cartera::ResumenBien>> {
        Ok(None)
    }
}
