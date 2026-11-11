//! Doble en memoria del directorio (plan 08, fase 1). Las fichas visibles en la copropiedad se
//! fijan en [`DatosDirectorio::fichas`] (la regla de cobertura es SQL y se prueba en
//! `infra-postgres`); aquí solo se filtran por anunciante, categoría, texto y destacadas.

use chrono::{DateTime, Utc};
use domain::{AnuncianteId, CuentaId, EstadoResena, ResenaId};

use super::TransaccionEnMemoria;
use crate::{
    AppError, AppResult, Pagina, SolicitudPagina,
    puertos::{
        RepositorioDirectorio,
        directorio::{Categoria, FichaDirectorio, FiltroDirectorio, ReporteResena, Resena},
    },
};

#[derive(Default, Clone)]
pub struct DatosDirectorio {
    pub fichas: Vec<FichaDirectorio>,
    pub categorias: Vec<Categoria>,
    /// Cuentas propietarias o residentes vigentes.
    pub residentes: Vec<CuentaId>,
    pub resenas: Vec<Resena>,
    /// Reportes con la fecha en que se atendieron.
    pub reportes: Vec<(ReporteResena, Option<DateTime<Utc>>)>,
}

impl RepositorioDirectorio for TransaccionEnMemoria {
    async fn fichas_directorio(
        &mut self,
        f: &FiltroDirectorio,
        _ahora: DateTime<Utc>,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<FichaDirectorio>> {
        let texto = f.texto.as_deref().map(str::to_lowercase);
        let mut v: Vec<FichaDirectorio> = self
            .datos
            .directorio
            .fichas
            .iter()
            .filter(|x| f.anunciante.is_none_or(|a| x.anunciante_id == a))
            .filter(|x| {
                f.categoria
                    .as_ref()
                    .is_none_or(|c| x.categorias.contains(c))
            })
            .filter(|x| {
                texto
                    .as_ref()
                    .is_none_or(|t| x.nombre_comercial.to_lowercase().contains(t))
            })
            .filter(|x| !f.solo_destacadas || x.destacada)
            .filter(|x| p.despues().is_none_or(|d| x.anunciante_id.as_uuid() > d))
            .cloned()
            .collect();
        v.sort_by_key(|x| x.anunciante_id);
        v.truncate(usize::try_from(p.filas_a_consultar()).unwrap_or(usize::MAX));
        Ok(Pagina::desde_filas(v, p, |x| x.anunciante_id.as_uuid()))
    }

    async fn categorias_directorio(&mut self) -> AppResult<Vec<Categoria>> {
        Ok(self
            .datos
            .directorio
            .categorias
            .iter()
            .filter(|c| c.activa && !c.restringida)
            .cloned()
            .collect())
    }

    async fn es_residente_vigente(
        &mut self,
        cuenta: CuentaId,
        _ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        Ok(self.datos.directorio.residentes.contains(&cuenta))
    }

    async fn resena_de_cuenta(
        &mut self,
        anunciante: AnuncianteId,
        cuenta: CuentaId,
    ) -> AppResult<Option<Resena>> {
        Ok(self
            .datos
            .directorio
            .resenas
            .iter()
            .find(|r| r.anunciante_id == anunciante && r.cuenta_id == cuenta)
            .cloned())
    }

    async fn resena(&mut self, id: ResenaId) -> AppResult<Option<Resena>> {
        Ok(self
            .datos
            .directorio
            .resenas
            .iter()
            .find(|r| r.id == id)
            .cloned())
    }

    async fn guardar_resena(&mut self, resena: &Resena) -> AppResult<()> {
        let v = &mut self.datos.directorio.resenas;
        v.retain(|r| r.id != resena.id);
        v.push(resena.clone());
        Ok(())
    }

    async fn resenas_publicadas(
        &mut self,
        anunciante: AnuncianteId,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<Resena>> {
        let mut v: Vec<Resena> = self
            .datos
            .directorio
            .resenas
            .iter()
            .filter(|r| r.anunciante_id == anunciante && r.estado == EstadoResena::Publicada)
            .filter(|r| p.despues().is_none_or(|d| r.id.as_uuid() < d))
            .cloned()
            .collect();
        v.sort_by_key(|r| std::cmp::Reverse(r.id));
        v.truncate(usize::try_from(p.filas_a_consultar()).unwrap_or(usize::MAX));
        Ok(Pagina::desde_filas(v, p, |r| r.id.as_uuid()))
    }

    async fn insertar_reporte(&mut self, reporte: &ReporteResena) -> AppResult<bool> {
        let v = &mut self.datos.directorio.reportes;
        if v.iter()
            .any(|(r, _)| r.resena_id == reporte.resena_id && r.cuenta_id == reporte.cuenta_id)
        {
            return Ok(false);
        }
        v.push((reporte.clone(), None));
        Ok(true)
    }

    async fn moderar_resena(
        &mut self,
        id: ResenaId,
        ocultar_con_motivo: Option<&str>,
        _moderador: Option<CuentaId>,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        let d = &mut self.datos.directorio;
        let r = d
            .resenas
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or(AppError::NoEncontrado("reseña"))?;
        if ocultar_con_motivo.is_some() {
            r.estado = EstadoResena::Oculta;
        }
        for (rep, atendido) in &mut d.reportes {
            if rep.resena_id == id && atendido.is_none() {
                *atendido = Some(ahora);
            }
        }
        Ok(())
    }
}
