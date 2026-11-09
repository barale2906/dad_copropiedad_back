//! «Mundo» de Typst en memoria: la plantilla como `main.typ`, los datos como `datos.json` y las
//! fuentes embebidas. No toca el disco ni la red.

use std::sync::LazyLock;

use application::{AppError, AppResult};
use typst::{
    Library, LibraryExt, World,
    diag::{FileError, FileResult},
    foundations::{Bytes, Datetime, Duration},
    syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot},
    text::{Font, FontBook},
    utils::LazyHash,
};
use typst_layout::PagedDocument;

struct Fuentes {
    libro: LazyHash<FontBook>,
    fuentes: Vec<Font>,
}

static FUENTES: LazyLock<Fuentes> = LazyLock::new(|| {
    let fuentes: Vec<Font> = typst_assets::fonts()
        .flat_map(|datos| Font::iter(Bytes::new(datos)))
        .collect();
    Fuentes {
        libro: LazyHash::new(FontBook::from_fonts(&fuentes)),
        fuentes,
    }
});

static BIBLIOTECA: LazyLock<LazyHash<Library>> =
    LazyLock::new(|| LazyHash::new(Library::default()));

fn id(nombre: &str) -> FileId {
    FileId::new(RootedPath::new(
        VirtualRoot::Project,
        VirtualPath::new(nombre).expect("ruta virtual fija y válida"),
    ))
}

struct Mundo {
    principal: Source,
    datos_id: FileId,
    datos: Bytes,
}

impl World for Mundo {
    fn library(&self) -> &LazyHash<Library> {
        &BIBLIOTECA
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &FUENTES.libro
    }

    fn main(&self) -> FileId {
        self.principal.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.principal.id() {
            Ok(self.principal.clone())
        } else {
            Err(FileError::AccessDenied)
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id == self.datos_id {
            Ok(self.datos.clone())
        } else if id == self.principal.id() {
            Ok(Bytes::from_string(self.principal.clone()))
        } else {
            Err(FileError::AccessDenied)
        }
    }

    fn font(&self, index: usize) -> Option<Font> {
        FUENTES.fuentes.get(index).cloned()
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        // Las plantillas no usan la fecha del sistema: la hora oficial viene en los datos.
        None
    }
}

/// Compila la plantilla con los datos y devuelve el PDF.
pub fn compilar(plantilla: &str, datos: Vec<u8>) -> AppResult<Vec<u8>> {
    let mundo = Mundo {
        principal: Source::new(id("main.typ"), plantilla.to_owned()),
        datos_id: id("datos.json"),
        datos: Bytes::new(datos),
    };
    let documento: PagedDocument = typst::compile(&mundo).output.map_err(|errores| {
        let mensajes: Vec<_> = errores.iter().map(|e| e.message.to_string()).collect();
        AppError::infra(std::io::Error::other(format!(
            "la plantilla Typst no compiló: {}",
            mensajes.join("; ")
        )))
    })?;
    typst_pdf::pdf(&documento, &typst_pdf::PdfOptions::default()).map_err(|errores| {
        let mensajes: Vec<_> = errores.iter().map(|e| e.message.to_string()).collect();
        AppError::infra(std::io::Error::other(format!(
            "no se pudo exportar el PDF: {}",
            mensajes.join("; ")
        )))
    })
}
