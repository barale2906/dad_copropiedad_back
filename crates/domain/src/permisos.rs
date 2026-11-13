//! Permisos y roles (estilo *spatie/laravel-permission* con *teams*: el "team" es la
//! copropiedad).
//!
//! - Hay dos **ámbitos**: permisos de **plataforma** (operar la plataforma: altas de PH,
//!   soporte, moderación) y de **copropiedad** (operar dentro de una PH). Un rol tiene permisos de
//!   un solo ámbito.
//! - El catálogo de permisos vive **en el código** ([`Permiso`]) y se sincroniza con la tabla
//!   `permisos` por migración (una prueba compara ambos).
//! - Los **roles plantilla** ([`ROLES_PLANTILLA`]) vienen con el sistema y no se editan; cada
//!   copropiedad puede crear roles propios con permisos de copropiedad, y el superusuario roles
//!   de plataforma propios.
//! - El **superusuario** tiene todos los permisos de ambos ámbitos en todas las copropiedades.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::DominioError;

/// Ámbito de un permiso o de un rol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ambito {
    Plataforma,
    Copropiedad,
}

enum_texto!(Ambito {
    Plataforma => "plataforma",
    Copropiedad => "copropiedad",
});

macro_rules! catalogo_permisos {
    ($( $(#[$meta:meta])* $variante:ident => $texto:literal, $ambito:ident; )+) => {
        /// Permiso del catálogo. El texto (`modulo.recurso.accion`) es contrato con la BD y con
        /// los clientes: no se renombra sin migración.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "&'static str")]
        pub enum Permiso {
            $( $(#[$meta])* $variante, )+
        }

        enum_texto!(Permiso { $( $variante => $texto, )+ });

        impl Permiso {
            /// Ámbito al que pertenece el permiso.
            pub const fn ambito(self) -> Ambito {
                match self {
                    $( Self::$variante => Ambito::$ambito, )+
                }
            }
        }
    };
}

catalogo_permisos! {
    // --- Plataforma ---
    /// Dar de alta copropiedades y asignar su primer administrador.
    PlataformaCopropiedadesCrear => "plataforma.copropiedades.crear", Plataforma;
    PlataformaCopropiedadesVer => "plataforma.copropiedades.ver", Plataforma;
    PlataformaCuentasVer => "plataforma.cuentas.ver", Plataforma;
    PlataformaCuentasBloquear => "plataforma.cuentas.bloquear", Plataforma;
    /// Crear y editar roles de plataforma propios.
    PlataformaRolesGestionar => "plataforma.roles.gestionar", Plataforma;
    PlataformaRolesAsignar => "plataforma.roles.asignar", Plataforma;
    /// Revisar reclamaciones de organizaciones y corregir fichas.
    PlataformaOrganizacionesRevisar => "plataforma.organizaciones.revisar", Plataforma;
    PlataformaModulosGestionar => "plataforma.modulos.gestionar", Plataforma;
    /// Dar de alta una entidad sin vertical (cualquier empresa de la contabilidad base).
    PlataformaEntidadesCrear => "plataforma.entidades.crear", Plataforma;
    /// Crear, publicar y retirar las plantillas contables estándar de la plataforma.
    PlataformaPlantillasContablesPublicar => "plataforma.plantillas_contables.publicar", Plataforma;
    /// Cargar catálogos normativos (PUC; más adelante UVT, retenciones, tasas).
    PlataformaNormativaGestionar => "plataforma.normativa.gestionar", Plataforma;
    PlataformaAuditoriaVer => "plataforma.auditoria.ver", Plataforma;
    PlataformaPublicidadModerar => "plataforma.publicidad.moderar", Plataforma;
    /// Tarifas de la pauta, política de publicidad de la plataforma y liquidaciones.
    PlataformaPublicidadLiquidar => "plataforma.publicidad.liquidar", Plataforma;
    /// Dar de alta anunciantes del directorio, editar sus fichas y el catálogo de categorías.
    PlataformaDirectorioGestionar => "plataforma.directorio.gestionar", Plataforma;
    PlataformaFacturacionVer => "plataforma.facturacion.ver", Plataforma;
    /// Líneas de WhatsApp (estado, franquicia) y tarifas de Meta.
    PlataformaWhatsappGestionar => "plataforma.whatsapp.gestionar", Plataforma;

    // --- Copropiedad ---
    CopropiedadDatosVer => "copropiedad.datos.ver", Copropiedad;
    CopropiedadDatosEditar => "copropiedad.datos.editar", Copropiedad;
    RolesVer => "roles.ver", Copropiedad;
    /// Crear y editar los roles propios de la copropiedad.
    RolesGestionar => "roles.gestionar", Copropiedad;
    RolesAsignar => "roles.asignar", Copropiedad;
    AuditoriaVer => "auditoria.ver", Copropiedad;
    ArchivosSubir => "archivos.subir", Copropiedad;
    /// Ver archivos públicos e internos.
    ArchivosVer => "archivos.ver", Copropiedad;
    /// Ver archivos con datos personales o sensibles.
    ArchivosPersonalesVer => "archivos.personales.ver", Copropiedad;
    PersonasVer => "personas.ver", Copropiedad;
    PersonasEditar => "personas.editar", Copropiedad;
    PersonasDatosSensiblesVer => "personas.datos_sensibles.ver", Copropiedad;
    /// Políticas de tratamiento, autorizaciones de terceros y solicitudes de titulares.
    ProteccionDatosGestionar => "proteccion_datos.gestionar", Copropiedad;
    OrganizacionesCrear => "organizaciones.crear", Copropiedad;
    OrganizacionesVincular => "organizaciones.vincular", Copropiedad;
    ContratosGestionar => "contratos.gestionar", Copropiedad;
    BienesPrivadosVer => "bienes_privados.ver", Copropiedad;
    BienesPrivadosEditar => "bienes_privados.editar", Copropiedad;
    /// Crear y llenar borradores de cuadros de coeficientes.
    BienesPrivadosCoeficientesEditar => "bienes_privados.coeficientes.editar", Copropiedad;
    /// Activar un cuadro de coeficientes (reforma del reglamento ya aprobada).
    BienesPrivadosCoeficientesActivar => "bienes_privados.coeficientes.activar", Copropiedad;
    /// Crear y editar edificios, etapas, bloques y demás agrupaciones.
    AgrupacionesEditar => "agrupaciones.editar", Copropiedad;
    /// Ver los titulares (propietarios) de los bienes privados.
    TitularesVer => "titulares.ver", Copropiedad;
    /// Registrar titulares, transferencias y representantes.
    TitularesEditar => "titulares.editar", Copropiedad;
    /// Ver residentes, vehículos y mascotas.
    CensoVer => "censo.ver", Copropiedad;
    /// Registrar y terminar residencias, vehículos y mascotas.
    CensoEditar => "censo.editar", Copropiedad;
    /// Aprobar o rechazar los cambios que proponen residentes y propietarios.
    CensoAprobar => "censo.aprobar", Copropiedad;
    /// Exportar los libros de propietarios y residentes (art. 51).
    CensoExportar => "censo.exportar", Copropiedad;
    /// Importar datos masivos desde CSV/XLSX.
    ImportacionesEjecutar => "importaciones.ejecutar", Copropiedad;
    BienesComunesVer => "bienes_comunes.ver", Copropiedad;
    BienesComunesEditar => "bienes_comunes.editar", Copropiedad;
    ReservasCrear => "reservas.crear", Copropiedad;
    ReservasAprobar => "reservas.aprobar", Copropiedad;
    /// Ver las reservas de todos los bienes privados.
    ReservasVerTodas => "reservas.ver_todas", Copropiedad;
    /// Cancelar reservas ajenas.
    ReservasCancelarCualquiera => "reservas.cancelar_cualquiera", Copropiedad;
    /// Registrar entrega, recibo y no asistencia (portería, conserjería).
    ReservasEntregasRegistrar => "reservas.entregas.registrar", Copropiedad;
    /// Restringir las reservas de un bien privado (mora o sanción, art. 59).
    ReservasRestringir => "reservas.restringir", Copropiedad;
    ParqueaderosVer => "parqueaderos.ver", Copropiedad;
    /// Crear parqueaderos y asignarlos o liberarlos.
    ParqueaderosAsignar => "parqueaderos.asignar", Copropiedad;
    /// Crear y ejecutar sorteos.
    SorteosGestionar => "sorteos.gestionar", Copropiedad;
    /// Radicar tickets propios y ver los que uno radicó.
    TicketsCrear => "tickets.crear", Copropiedad;
    /// Clasificar, asignar, rechazar, marcar duplicados y cerrar tickets.
    TicketsAsignar => "tickets.asignar", Copropiedad;
    /// Trabajar los tickets asignados (a la cuenta o a su organización) y escribir notas internas.
    TicketsAtender => "tickets.atender", Copropiedad;
    /// Ver todos los tickets de la copropiedad (sin la identidad de quien reporta convivencia).
    TicketsVerTodos => "tickets.ver_todos", Copropiedad;
    /// Categorías, jornada hábil y plazos de respuesta.
    TicketsConfigurar => "tickets.configurar", Copropiedad;
    /// Ver y gestionar los reportes de convivencia con la identidad del denunciante (art. 58).
    TicketsConvivenciaVer => "tickets.convivencia.ver", Copropiedad;
    /// Recibir de inmediato los reportes urgentes (fugas, ascensor atrapado…).
    TicketsUrgenciasRecibir => "tickets.urgencias.recibir", Copropiedad;
    /// Atender en la web las conversaciones de WhatsApp escaladas a la administración.
    ConversacionesAtender => "conversaciones.atender", Copropiedad;
    /// Editar las preguntas frecuentes que responde el chat.
    PreguntasFrecuentesEditar => "preguntas_frecuentes.editar", Copropiedad;
    InventarioVer => "inventario.ver", Copropiedad;
    /// Registrar activos, movimientos y bajas.
    InventarioEditar => "inventario.editar", Copropiedad;
    /// Planes preventivos, órdenes de trabajo y obligaciones periódicas.
    MantenimientoPlanificar => "mantenimiento.planificar", Copropiedad;
    /// Ejecutar las órdenes asignadas (checklist, evidencias, costos).
    MantenimientoEjecutar => "mantenimiento.ejecutar", Copropiedad;
    /// Verificar (o devolver) las órdenes ejecutadas.
    MantenimientoVerificar => "mantenimiento.verificar", Copropiedad;
    AsambleasVer => "asambleas.ver", Copropiedad;
    AsambleasConvocar => "asambleas.convocar", Copropiedad;
    AsambleasConducir => "asambleas.conducir", Copropiedad;
    /// Mesa de registro: asistentes, poderes, rótulos, salidas y reingresos.
    AsambleasRegistrar => "asambleas.registrar", Copropiedad;
    /// Registrar votos asistidos (operador) y desde los puestos de escaneo de rótulos.
    AsambleasVotosRegistrar => "asambleas.votos.registrar", Copropiedad;
    /// Ver qué votó cada bien privado en las votaciones reservadas (plan 06, ADR 0010).
    AsambleasVotosVerDetalle => "asambleas.votos.ver_detalle", Copropiedad;
    AccesoRegistrar => "acceso.registrar", Copropiedad;
    AccesoVisitantesAutorizar => "acceso.visitantes.autorizar", Copropiedad;
    AccesoBitacoraVer => "acceso.bitacora.ver", Copropiedad;
    /// Entregar correspondencia (con firma o foto de quien recoge) y devolverla.
    AccesoCorrespondenciaEntregar => "acceso.correspondencia.entregar", Copropiedad;
    AccesoMinutaRegistrar => "acceso.minuta.registrar", Copropiedad;
    AccesoMinutaVer => "acceso.minuta.ver", Copropiedad;
    /// Configuración del módulo: plazos, silencio, puntos de acceso, puentes y dispositivos.
    AccesoConfigurar => "acceso.configurar", Copropiedad;
    /// Emitir, suspender y revocar credenciales; autorizaciones manuales.
    AccesoCredencialesGestionar => "acceso.credenciales.gestionar", Copropiedad;
    /// Enrolar biometría (solo con consentimiento vigente).
    AccesoBiometriaEnrolar => "acceso.biometria.enrolar", Copropiedad;
    /// Redactar, programar, enviar y anular comunicados; ver su tablero de lectura.
    ComunicadosGestionar => "comunicados.gestionar", Copropiedad;
    ContabilidadVer => "contabilidad.ver", Copropiedad;
    /// Configuración contable, plan de cuentas, centros de costo, tipos de comprobante, reglas,
    /// rubros y aplicar o guardar plantillas.
    ContabilidadConfigurar => "contabilidad.configurar", Copropiedad;
    /// Datos fiscales de los terceros (personas de la entidad).
    ContabilidadTercerosGestionar => "contabilidad.terceros.gestionar", Copropiedad;
    /// Crear y editar comprobantes en borrador.
    ContabilidadComprobantesCrear => "contabilidad.comprobantes.crear", Copropiedad;
    /// Contabilizar comprobantes, saldos iniciales y cierre anual.
    ContabilidadComprobantesContabilizar => "contabilidad.comprobantes.contabilizar", Copropiedad;
    /// Revertir un comprobante contabilizado.
    ContabilidadComprobantesRevertir => "contabilidad.comprobantes.revertir", Copropiedad;
    /// Abrir periodos, pasarlos a revisión y cerrarlos.
    ContabilidadPeriodosCerrar => "contabilidad.periodos.cerrar", Copropiedad;
    /// Solicitar la reapertura de un periodo cerrado.
    ContabilidadPeriodosReabrir => "contabilidad.periodos.reabrir", Copropiedad;
    /// Aprobar o rechazar la reapertura que pidió otra cuenta.
    ContabilidadPeriodosAprobarReapertura => "contabilidad.periodos.aprobar_reapertura", Copropiedad;
    /// Exportar libros y estados financieros (CSV).
    ContabilidadExportar => "contabilidad.exportar", Copropiedad;
    /// Cuentas de tesorería (bancos, cajas y fondos).
    TesoreriaConfigurar => "tesoreria.configurar", Copropiedad;
    /// Registrar, aplicar y anular recibos de caja.
    TesoreriaRecibosRegistrar => "tesoreria.recibos.registrar", Copropiedad;
    /// Registrar, pagar y anular egresos.
    TesoreriaEgresosRegistrar => "tesoreria.egresos.registrar", Copropiedad;
    /// Autorizar egresos que registró otra cuenta y la excepción de una factura duplicada.
    TesoreriaEgresosAutorizar => "tesoreria.egresos.autorizar", Copropiedad;
    /// Registrar, causar y anular facturas de proveedores; certificados de retención.
    TesoreriaFacturasProveedorGestionar => "tesoreria.facturas_proveedor.gestionar", Copropiedad;
    /// Crear, emitir y anular facturas de venta.
    TesoreriaFacturasVentaGestionar => "tesoreria.facturas_venta.gestionar", Copropiedad;
    /// Importar extractos y conciliar.
    TesoreriaConciliar => "tesoreria.conciliar", Copropiedad;
    /// Consultar el directorio de comercios y servicios que alcanzan a la copropiedad.
    DirectorioVer => "directorio.ver", Copropiedad;
    /// Calificar a un anunciante del directorio (además, ser residente o propietario vigente).
    DirectorioResenar => "directorio.resenar", Copropiedad;
    /// Generar o revocar el enlace público del directorio para los grupos de WhatsApp.
    DirectorioCompartir => "directorio.compartir", Copropiedad;
}

impl TryFrom<String> for Permiso {
    type Error = DominioError;

    fn try_from(valor: String) -> Result<Self, Self::Error> {
        Self::try_from(valor.as_str())
    }
}

impl From<Permiso> for &'static str {
    fn from(p: Permiso) -> Self {
        p.as_str()
    }
}

/// Conjunto de permisos efectivos (unión de los roles vigentes). Inmutable una vez armado.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConjuntoPermisos(BTreeSet<Permiso>);

impl ConjuntoPermisos {
    /// Conjunto vacío: no puede nada.
    pub fn vacio() -> Self {
        Self::default()
    }

    /// Todos los permisos de un ámbito.
    pub fn todos(ambito: Ambito) -> Self {
        Self(
            Permiso::TODOS
                .iter()
                .copied()
                .filter(|p| p.ambito() == ambito)
                .collect(),
        )
    }

    /// Si el conjunto tiene el permiso.
    pub fn contiene(&self, permiso: Permiso) -> bool {
        self.0.contains(&permiso)
    }

    /// ¿Todos los permisos de `otro` están aquí? (Regla de no escalamiento: nadie otorga lo que
    /// no tiene.)
    pub fn incluye(&self, otro: &Self) -> bool {
        otro.0.is_subset(&self.0)
    }

    /// Si el conjunto no tiene permisos.
    pub fn es_vacio(&self) -> bool {
        self.0.is_empty()
    }

    /// Permisos del conjunto.
    pub fn iter(&self) -> impl Iterator<Item = Permiso> + '_ {
        self.0.iter().copied()
    }

    /// Valida que todos los permisos sean del ámbito.
    ///
    /// # Errors
    ///
    /// [`DominioError::PermisoDeOtroAmbito`] con el primer permiso que no corresponde.
    pub fn exigir_ambito(&self, ambito: Ambito) -> Result<(), DominioError> {
        match self.0.iter().find(|p| p.ambito() != ambito) {
            Some(p) => Err(DominioError::PermisoDeOtroAmbito(*p)),
            None => Ok(()),
        }
    }
}

impl FromIterator<Permiso> for ConjuntoPermisos {
    fn from_iter<I: IntoIterator<Item = Permiso>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

/// Código del rol plantilla con acceso total.
pub const CODIGO_SUPERUSUARIO: &str = "superusuario";
/// Código del rol plantilla del representante legal de la copropiedad (Ley 675, art. 50).
pub const CODIGO_ADMINISTRADOR: &str = "administrador";

/// Rol que viene con el sistema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RolPlantilla {
    pub codigo: &'static str,
    pub nombre: &'static str,
    pub ambito: Ambito,
    /// `None` = todos los permisos del ámbito.
    pub permisos: Option<&'static [Permiso]>,
}

impl RolPlantilla {
    /// Permisos efectivos del rol.
    pub fn conjunto(&self) -> ConjuntoPermisos {
        match self.permisos {
            None => ConjuntoPermisos::todos(self.ambito),
            Some(p) => p.iter().copied().collect(),
        }
    }
}

use Permiso as P;

/// Roles plantilla. **Contrato con la migración** `identidad` (prueba
/// `roles_plantilla_coinciden_con_el_codigo`): cambiar uno exige una migración.
pub const ROLES_PLANTILLA: &[RolPlantilla] = &[
    RolPlantilla {
        codigo: CODIGO_SUPERUSUARIO,
        nombre: "Superusuario",
        ambito: Ambito::Plataforma,
        permisos: None,
    },
    RolPlantilla {
        codigo: "soporte",
        nombre: "Soporte",
        ambito: Ambito::Plataforma,
        permisos: Some(&[
            P::PlataformaCopropiedadesVer,
            P::PlataformaCuentasVer,
            P::PlataformaCuentasBloquear,
            P::PlataformaOrganizacionesRevisar,
            P::PlataformaAuditoriaVer,
        ]),
    },
    RolPlantilla {
        codigo: "comercial",
        nombre: "Comercial",
        ambito: Ambito::Plataforma,
        permisos: Some(&[
            P::PlataformaCopropiedadesCrear,
            P::PlataformaEntidadesCrear,
            P::PlataformaCopropiedadesVer,
            P::PlataformaModulosGestionar,
            P::PlataformaDirectorioGestionar,
            P::PlataformaPublicidadLiquidar,
        ]),
    },
    RolPlantilla {
        codigo: "moderador_publicidad",
        nombre: "Moderador de publicidad",
        ambito: Ambito::Plataforma,
        permisos: Some(&[
            P::PlataformaPublicidadModerar,
            P::PlataformaDirectorioGestionar,
        ]),
    },
    RolPlantilla {
        codigo: "facturacion",
        nombre: "Facturación",
        ambito: Ambito::Plataforma,
        permisos: Some(&[
            P::PlataformaCopropiedadesVer,
            P::PlataformaFacturacionVer,
            P::PlataformaPublicidadLiquidar,
        ]),
    },
    RolPlantilla {
        codigo: CODIGO_ADMINISTRADOR,
        nombre: "Administrador",
        ambito: Ambito::Copropiedad,
        permisos: None,
    },
    RolPlantilla {
        codigo: "asistente_administrativo",
        nombre: "Asistente administrativo",
        ambito: Ambito::Copropiedad,
        permisos: Some(&[
            P::CopropiedadDatosVer,
            P::RolesVer,
            P::ArchivosSubir,
            P::ArchivosVer,
            P::PersonasVer,
            P::PersonasEditar,
            P::OrganizacionesCrear,
            P::OrganizacionesVincular,
            P::BienesPrivadosVer,
            P::TitularesVer,
            P::CensoVer,
            P::CensoEditar,
            P::CensoAprobar,
            P::BienesComunesVer,
            P::ReservasAprobar,
            P::ReservasVerTodas,
            P::ReservasCancelarCualquiera,
            P::ReservasEntregasRegistrar,
            P::ParqueaderosVer,
            P::TicketsCrear,
            P::TicketsAsignar,
            P::TicketsVerTodos,
            P::TicketsAtender,
            P::ConversacionesAtender,
            P::PreguntasFrecuentesEditar,
            P::InventarioVer,
            P::InventarioEditar,
            P::MantenimientoPlanificar,
            P::MantenimientoVerificar,
            P::AsambleasVer,
            P::AsambleasRegistrar,
            P::AccesoBitacoraVer,
            P::ComunicadosGestionar,
            P::DirectorioVer,
            P::DirectorioCompartir,
        ]),
    },
    RolPlantilla {
        codigo: "contador",
        nombre: "Contador",
        ambito: Ambito::Copropiedad,
        permisos: Some(&[
            P::CopropiedadDatosVer,
            P::ArchivosVer,
            P::ArchivosSubir,
            P::BienesPrivadosVer,
            P::ContabilidadVer,
            P::ContabilidadConfigurar,
            P::ContabilidadTercerosGestionar,
            P::ContabilidadComprobantesCrear,
            P::ContabilidadComprobantesContabilizar,
            P::ContabilidadComprobantesRevertir,
            P::ContabilidadPeriodosCerrar,
            P::ContabilidadPeriodosReabrir,
            P::ContabilidadExportar,
            P::TesoreriaConfigurar,
            P::TesoreriaRecibosRegistrar,
            P::TesoreriaEgresosRegistrar,
            P::TesoreriaFacturasProveedorGestionar,
            P::TesoreriaFacturasVentaGestionar,
            P::TesoreriaConciliar,
        ]),
    },
    RolPlantilla {
        codigo: "revisor_fiscal",
        nombre: "Revisor fiscal",
        ambito: Ambito::Copropiedad,
        permisos: Some(&[
            P::CopropiedadDatosVer,
            P::ArchivosVer,
            P::AuditoriaVer,
            P::BienesPrivadosVer,
            P::TitularesVer,
            P::ContabilidadVer,
            P::ContabilidadExportar,
            P::AsambleasVer,
            P::InventarioVer,
        ]),
    },
    RolPlantilla {
        codigo: "consejero",
        nombre: "Consejero",
        ambito: Ambito::Copropiedad,
        permisos: Some(&[
            P::CopropiedadDatosVer,
            P::ArchivosVer,
            P::BienesPrivadosVer,
            P::BienesComunesVer,
            P::AsambleasVer,
            P::ContabilidadVer,
            P::DirectorioVer,
        ]),
    },
    RolPlantilla {
        codigo: "presidente_consejo",
        nombre: "Presidente del consejo",
        ambito: Ambito::Copropiedad,
        permisos: Some(&[
            P::CopropiedadDatosVer,
            P::ArchivosVer,
            P::BienesPrivadosVer,
            P::BienesComunesVer,
            P::AsambleasVer,
            P::AsambleasConvocar,
            P::ContabilidadVer,
            P::TesoreriaEgresosAutorizar,
            P::TicketsVerTodos,
            P::DirectorioVer,
        ]),
    },
    RolPlantilla {
        codigo: "comite_convivencia",
        nombre: "Comité de convivencia",
        ambito: Ambito::Copropiedad,
        permisos: Some(&[
            P::CopropiedadDatosVer,
            P::TicketsAtender,
            P::TicketsConvivenciaVer,
            P::ArchivosVer,
        ]),
    },
    RolPlantilla {
        codigo: "propietario",
        nombre: "Propietario",
        ambito: Ambito::Copropiedad,
        permisos: Some(&[
            P::CopropiedadDatosVer,
            P::ArchivosVer,
            P::BienesComunesVer,
            P::ReservasCrear,
            P::TicketsCrear,
            P::AsambleasVer,
            P::AccesoVisitantesAutorizar,
            P::DirectorioVer,
            P::DirectorioResenar,
        ]),
    },
    RolPlantilla {
        codigo: "residente",
        nombre: "Residente",
        ambito: Ambito::Copropiedad,
        permisos: Some(&[
            P::CopropiedadDatosVer,
            P::ArchivosVer,
            P::BienesComunesVer,
            P::ReservasCrear,
            P::TicketsCrear,
            P::AccesoVisitantesAutorizar,
            P::DirectorioVer,
            P::DirectorioResenar,
        ]),
    },
    RolPlantilla {
        codigo: "vigilante",
        nombre: "Vigilante / portero",
        ambito: Ambito::Copropiedad,
        permisos: Some(&[
            P::AccesoRegistrar,
            P::AccesoBitacoraVer,
            P::AccesoCorrespondenciaEntregar,
            P::AccesoMinutaRegistrar,
            P::AccesoMinutaVer,
            P::TicketsCrear,
            P::ReservasEntregasRegistrar,
            P::ParqueaderosVer,
        ]),
    },
    RolPlantilla {
        codigo: "supervisor_seguridad",
        nombre: "Supervisor de seguridad",
        ambito: Ambito::Copropiedad,
        permisos: Some(&[
            P::AccesoRegistrar,
            P::AccesoBitacoraVer,
            P::AccesoVisitantesAutorizar,
            P::AccesoCorrespondenciaEntregar,
            P::AccesoMinutaRegistrar,
            P::AccesoMinutaVer,
            P::AccesoCredencialesGestionar,
            P::TicketsCrear,
            P::ReservasEntregasRegistrar,
            P::ParqueaderosVer,
            P::TicketsUrgenciasRecibir,
        ]),
    },
    RolPlantilla {
        codigo: "aseo",
        nombre: "Personal de aseo",
        ambito: Ambito::Copropiedad,
        permisos: Some(&[P::TicketsAtender]),
    },
    RolPlantilla {
        codigo: "mantenimiento",
        nombre: "Mantenimiento",
        ambito: Ambito::Copropiedad,
        permisos: Some(&[
            P::TicketsAtender,
            P::BienesComunesVer,
            P::TicketsUrgenciasRecibir,
            P::MantenimientoEjecutar,
            P::InventarioVer,
        ]),
    },
    RolPlantilla {
        codigo: "operador_asamblea",
        nombre: "Operador de asamblea",
        ambito: Ambito::Copropiedad,
        permisos: Some(&[
            P::AsambleasVer,
            P::AsambleasRegistrar,
            P::AsambleasVotosRegistrar,
        ]),
    },
    RolPlantilla {
        codigo: "proveedor_tecnico",
        nombre: "Proveedor técnico",
        ambito: Ambito::Copropiedad,
        permisos: Some(&[P::TicketsAtender, P::MantenimientoEjecutar]),
    },
];

/// Origen de una asignación de rol. Las derivadas no se asignan a mano: nacen y vencen con el
/// hecho que las origina (vender el bien privado retira el acceso del vendedor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrigenAsignacion {
    Manual,
    /// Personal de una organización con contrato de servicio vigente.
    ContratoServicio,
    /// Titularidad de un bien privado (plan 03).
    Propiedad,
    /// Residencia registrada en el censo (plan 07).
    Residencia,
    /// Elección en asamblea (consejo, revisor fiscal; plan 06).
    Asamblea,
}

enum_texto!(OrigenAsignacion {
    Manual => "manual",
    ContratoServicio => "contrato_servicio",
    Propiedad => "propiedad",
    Residencia => "residencia",
    Asamblea => "asamblea",
});

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn textos_unicos_y_con_formato() {
        let mut vistos = HashSet::new();
        for p in Permiso::TODOS {
            assert!(vistos.insert(p.as_str()), "duplicado: {p}");
            assert!(
                p.as_str()
                    .split('.')
                    .all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c == '_')),
                "{p}"
            );
            assert_eq!(Permiso::try_from(p.as_str()), Ok(*p));
            let es_plataforma = p.as_str().starts_with("plataforma.");
            assert_eq!(es_plataforma, p.ambito() == Ambito::Plataforma, "{p}");
        }
    }

    #[test]
    fn todos_separa_por_ambito() {
        let plataforma = ConjuntoPermisos::todos(Ambito::Plataforma);
        let copropiedad = ConjuntoPermisos::todos(Ambito::Copropiedad);
        assert!(plataforma.contiene(Permiso::PlataformaCopropiedadesCrear));
        assert!(!plataforma.contiene(Permiso::RolesAsignar));
        assert!(copropiedad.contiene(Permiso::AsambleasVotosVerDetalle));
        assert!(copropiedad.exigir_ambito(Ambito::Copropiedad).is_ok());
        assert_eq!(
            plataforma.exigir_ambito(Ambito::Copropiedad),
            Err(DominioError::PermisoDeOtroAmbito(
                Permiso::PlataformaCopropiedadesCrear
            ))
        );
    }

    #[test]
    fn no_escalamiento_por_inclusion() {
        let asistente = ROLES_PLANTILLA
            .iter()
            .find(|r| r.codigo == "asistente_administrativo")
            .expect("existe")
            .conjunto();
        let admin = ConjuntoPermisos::todos(Ambito::Copropiedad);

        assert!(admin.incluye(&asistente));
        assert!(!asistente.incluye(&admin));
        assert!(asistente.incluye(&ConjuntoPermisos::vacio()));
    }

    #[test]
    fn plantillas_unicas_y_con_permisos_de_su_ambito() {
        let mut codigos = HashSet::new();
        for rol in ROLES_PLANTILLA {
            assert!(codigos.insert(rol.codigo), "duplicado: {}", rol.codigo);
            assert!(
                rol.conjunto().exigir_ambito(rol.ambito).is_ok(),
                "{}",
                rol.codigo
            );
            assert!(!rol.conjunto().es_vacio(), "{}", rol.codigo);
        }
        assert!(codigos.contains(CODIGO_SUPERUSUARIO) && codigos.contains(CODIGO_ADMINISTRADOR));
    }

    #[test]
    fn serde_usa_el_texto_del_permiso() {
        let json = serde_json::to_string(&Permiso::RolesAsignar).expect("serializa");
        assert_eq!(json, "\"roles.asignar\"");
        assert!(serde_json::from_str::<Permiso>("\"no.existe\"").is_err());
    }
}
