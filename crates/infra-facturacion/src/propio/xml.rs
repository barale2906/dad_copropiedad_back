//! Árbol XML mínimo y su forma canónica (C14N 1.0 inclusiva y exclusiva, sin comentarios).
//!
//! La firma XMLDSig se calcula sobre la forma canónica de partes del documento. En lugar de un
//! canonicalizador general (no hay uno maduro en Rust puro), el motor arma los documentos con
//! este árbol y los serializa **siempre** en forma canónica: así lo que se firma es exactamente lo
//! que se envía. Solo cubre lo que el motor genera: elementos, atributos, declaraciones de
//! espacios de nombres y texto (sin comentarios, instrucciones de proceso ni DTD).

use std::collections::BTreeMap;

/// Nodo hijo de un elemento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Nodo {
    Elemento(Elemento),
    Texto(String),
}

/// Elemento con su nombre calificado (`cbc:ID`), sus declaraciones de espacios de nombres,
/// atributos e hijos, en el orden en que se escriben.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Elemento {
    pub(crate) nombre: String,
    /// Declaraciones `xmlns:prefijo="uri"` propias del elemento (prefijo vacío = por defecto).
    pub(crate) espacios: Vec<(String, String)>,
    pub(crate) atributos: Vec<(String, String)>,
    pub(crate) hijos: Vec<Nodo>,
}

impl Elemento {
    pub(crate) fn new(nombre: &str) -> Self {
        Self {
            nombre: nombre.to_owned(),
            espacios: Vec::new(),
            atributos: Vec::new(),
            hijos: Vec::new(),
        }
    }

    /// Declara un espacio de nombres en este elemento.
    pub(crate) fn ns(mut self, prefijo: &str, uri: &str) -> Self {
        self.espacios.push((prefijo.to_owned(), uri.to_owned()));
        self
    }

    pub(crate) fn attr(mut self, nombre: &str, valor: impl Into<String>) -> Self {
        self.atributos.push((nombre.to_owned(), valor.into()));
        self
    }

    pub(crate) fn texto(mut self, texto: impl Into<String>) -> Self {
        self.hijos.push(Nodo::Texto(texto.into()));
        self
    }

    pub(crate) fn hijo(mut self, hijo: Elemento) -> Self {
        self.hijos.push(Nodo::Elemento(hijo));
        self
    }

    pub(crate) fn hijo_si(self, hijo: Option<Elemento>) -> Self {
        match hijo {
            Some(h) => self.hijo(h),
            None => self,
        }
    }

    pub(crate) fn hijos(mut self, hijos: impl IntoIterator<Item = Elemento>) -> Self {
        self.hijos.extend(hijos.into_iter().map(Nodo::Elemento));
        self
    }

    /// Elemento hoja con texto: `<nombre>texto</nombre>`.
    pub(crate) fn hoja(nombre: &str, texto: impl Into<String>) -> Self {
        Self::new(nombre).texto(texto)
    }

    /// Primer descendiente (o él mismo) con ese nombre, en profundidad.
    pub(crate) fn buscar_mut(&mut self, nombre: &str) -> Option<&mut Elemento> {
        if self.nombre == nombre {
            return Some(self);
        }
        self.hijos.iter_mut().find_map(|h| match h {
            Nodo::Elemento(e) => e.buscar_mut(nombre),
            Nodo::Texto(_) => None,
        })
    }

    /// Último elemento hijo directo con ese nombre.
    pub(crate) fn ultimo_hijo_mut(&mut self, nombre: &str) -> Option<&mut Elemento> {
        self.hijos.iter_mut().rev().find_map(|h| match h {
            Nodo::Elemento(e) if e.nombre == nombre => Some(e),
            _ => None,
        })
    }
}

/// Variante de la canonicalización.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Canon {
    /// C14N 1.0 (`http://www.w3.org/TR/2001/REC-xml-c14n-20010315`): el elemento raíz del
    /// subconjunto lleva todos los espacios de nombres en alcance. La exige la DIAN en la firma
    /// de los documentos.
    Inclusiva,
    /// C14N exclusiva (`http://www.w3.org/2001/10/xml-exc-c14n#`): solo los espacios de nombres
    /// que el elemento usa. La usa la firma WS-Security de los servicios web.
    Exclusiva,
}

/// Forma canónica de `elemento` como raíz de un subconjunto del documento. `heredados` son los
/// espacios de nombres declarados en sus ancestros (en orden de declaración).
pub(crate) fn canonica(elemento: &Elemento, heredados: &[(String, String)], modo: Canon) -> String {
    let alcance: BTreeMap<String, String> = heredados.iter().cloned().collect();
    let mut salida = String::new();
    escribir(elemento, &alcance, &BTreeMap::new(), modo, &mut salida);
    salida
}

/// Documento completo: declaración XML y la raíz en forma canónica.
pub(crate) fn documento(raiz: &Elemento) -> Vec<u8> {
    let mut salida = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>");
    salida.push_str(&canonica(raiz, &[], Canon::Inclusiva));
    salida.into_bytes()
}

fn prefijo(nombre: &str) -> &str {
    nombre.split_once(':').map_or("", |(p, _)| p)
}

fn local(nombre: &str) -> &str {
    nombre.split_once(':').map_or(nombre, |(_, l)| l)
}

/// Prefijos que el elemento usa en su nombre y en sus atributos (C14N exclusiva). Un atributo
/// sin prefijo no pertenece a ningún espacio de nombres, ni siquiera al por defecto.
fn utilizados(elemento: &Elemento) -> Vec<&str> {
    let mut prefijos = vec![prefijo(&elemento.nombre)];
    for (nombre, _) in &elemento.atributos {
        if nombre.contains(':') {
            prefijos.push(prefijo(nombre));
        }
    }
    prefijos
}

fn escribir(
    elemento: &Elemento,
    alcance_padre: &BTreeMap<String, String>,
    renderizados: &BTreeMap<String, String>,
    modo: Canon,
    salida: &mut String,
) {
    let mut alcance = alcance_padre.clone();
    for (p, u) in &elemento.espacios {
        alcance.insert(p.clone(), u.clone());
    }
    let candidatos: Vec<&str> = match modo {
        Canon::Inclusiva => alcance.keys().map(String::as_str).collect(),
        Canon::Exclusiva => utilizados(elemento),
    };
    // BTreeMap ordena por prefijo, con el espacio por defecto ("") primero, como pide C14N.
    let mut emitir: BTreeMap<&str, &str> = BTreeMap::new();
    for p in candidatos {
        if let Some(u) = alcance.get(p)
            && !(p.is_empty() && u.is_empty() && !renderizados.contains_key(""))
            && renderizados.get(p) != Some(u)
        {
            emitir.insert(p, u);
        }
    }
    let mut nuevos = renderizados.clone();
    for (p, u) in &emitir {
        nuevos.insert((*p).to_owned(), (*u).to_owned());
    }

    salida.push('<');
    salida.push_str(&elemento.nombre);
    for (p, u) in &emitir {
        if p.is_empty() {
            salida.push_str(" xmlns=\"");
        } else {
            salida.push_str(" xmlns:");
            salida.push_str(p);
            salida.push_str("=\"");
        }
        escapar_atributo(u, salida);
        salida.push('"');
    }
    // Atributos ordenados por (URI del espacio de nombres, nombre local); los que no tienen
    // prefijo no tienen espacio de nombres y van primero.
    let mut atributos: Vec<(&str, &str, &str, &str)> = elemento
        .atributos
        .iter()
        .map(|(n, v)| {
            let uri = if n.contains(':') {
                alcance.get(prefijo(n)).map_or("", String::as_str)
            } else {
                ""
            };
            (uri, local(n), n.as_str(), v.as_str())
        })
        .collect();
    atributos.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    for (_, _, nombre, valor) in atributos {
        salida.push(' ');
        salida.push_str(nombre);
        salida.push_str("=\"");
        escapar_atributo(valor, salida);
        salida.push('"');
    }
    salida.push('>');
    for hijo in &elemento.hijos {
        match hijo {
            Nodo::Elemento(e) => escribir(e, &alcance, &nuevos, modo, salida),
            Nodo::Texto(t) => escapar_texto(t, salida),
        }
    }
    salida.push_str("</");
    salida.push_str(&elemento.nombre);
    salida.push('>');
}

fn escapar_texto(texto: &str, salida: &mut String) {
    for c in texto.chars() {
        match c {
            '&' => salida.push_str("&amp;"),
            '<' => salida.push_str("&lt;"),
            '>' => salida.push_str("&gt;"),
            '\r' => salida.push_str("&#xD;"),
            otro => salida.push(otro),
        }
    }
}

fn escapar_atributo(texto: &str, salida: &mut String) {
    for c in texto.chars() {
        match c {
            '&' => salida.push_str("&amp;"),
            '<' => salida.push_str("&lt;"),
            '"' => salida.push_str("&quot;"),
            '\t' => salida.push_str("&#x9;"),
            '\n' => salida.push_str("&#xA;"),
            '\r' => salida.push_str("&#xD;"),
            otro => salida.push(otro),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raiz() -> Elemento {
        Elemento::new("Invoice")
            .ns("", "urn:invoice")
            .ns("cbc", "urn:cbc")
            .ns("ds", "urn:ds")
            .hijo(
                Elemento::new("cbc:ID")
                    .attr("schemeName", "31")
                    .attr("schemeID", "4")
                    .texto("A&B<C>"),
            )
            .hijo(
                Elemento::new("ds:KeyInfo")
                    .attr("Id", "k1")
                    .hijo(Elemento::hoja("ds:X", "1")),
            )
    }

    #[test]
    fn documento_completo_ordena_atributos_y_escapa_el_texto() {
        let xml = String::from_utf8(documento(&raiz())).expect("utf-8");
        assert_eq!(
            xml,
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\
             <Invoice xmlns=\"urn:invoice\" xmlns:cbc=\"urn:cbc\" xmlns:ds=\"urn:ds\">\
             <cbc:ID schemeID=\"4\" schemeName=\"31\">A&amp;B&lt;C&gt;</cbc:ID>\
             <ds:KeyInfo Id=\"k1\"><ds:X>1</ds:X></ds:KeyInfo></Invoice>"
        );
    }

    #[test]
    fn inclusiva_de_un_subconjunto_lleva_todos_los_espacios_heredados() {
        let r = raiz();
        let Nodo::Elemento(key_info) = &r.hijos[1] else {
            panic!("se esperaba un elemento");
        };
        assert_eq!(
            canonica(key_info, &r.espacios, Canon::Inclusiva),
            "<ds:KeyInfo xmlns=\"urn:invoice\" xmlns:cbc=\"urn:cbc\" xmlns:ds=\"urn:ds\" Id=\"k1\">\
             <ds:X>1</ds:X></ds:KeyInfo>"
        );
    }

    #[test]
    fn exclusiva_solo_lleva_los_espacios_que_usa() {
        let r = raiz();
        let Nodo::Elemento(key_info) = &r.hijos[1] else {
            panic!("se esperaba un elemento");
        };
        assert_eq!(
            canonica(key_info, &r.espacios, Canon::Exclusiva),
            "<ds:KeyInfo xmlns:ds=\"urn:ds\" Id=\"k1\"><ds:X>1</ds:X></ds:KeyInfo>"
        );
    }

    #[test]
    fn atributos_con_prefijo_se_ordenan_por_uri_despues_de_los_simples() {
        let e = Elemento::new("a:E")
            .ns("a", "urn:z")
            .ns("b", "urn:a")
            .attr("a:x", "1")
            .attr("b:y", "2")
            .attr("z", "3");
        assert_eq!(
            canonica(&e, &[], Canon::Inclusiva),
            "<a:E xmlns:a=\"urn:z\" xmlns:b=\"urn:a\" z=\"3\" b:y=\"2\" a:x=\"1\"></a:E>"
        );
    }

    #[test]
    fn un_espacio_redeclarado_igual_no_se_repite_en_el_hijo() {
        let e = Elemento::new("a:E")
            .ns("a", "urn:a")
            .hijo(Elemento::new("a:H").ns("a", "urn:a"));
        assert_eq!(
            canonica(&e, &[], Canon::Inclusiva),
            "<a:E xmlns:a=\"urn:a\"><a:H></a:H></a:E>"
        );
    }

    #[test]
    fn atributos_escapan_comillas_y_saltos() {
        let e = Elemento::new("E").attr("v", "a\"b\nc&");
        assert_eq!(
            canonica(&e, &[], Canon::Inclusiva),
            "<E v=\"a&quot;b&#xA;c&amp;\"></E>"
        );
    }

    #[test]
    fn buscar_encuentra_el_primer_descendiente() {
        let mut r = raiz();
        assert!(r.buscar_mut("ds:X").is_some());
        assert!(r.buscar_mut("ds:Nada").is_none());
    }
}
