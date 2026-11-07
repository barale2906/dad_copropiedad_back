# Genera los XLSX de prueba: python3 -I generar.py <carpeta>  (solo biblioteca estándar).
import sys, zipfile
from xml.sax.saxutils import escape

def celda(ref, v):
    if v is None or v == "":
        return ""
    if isinstance(v, (int, float)):
        return f'<c r="{ref}"><v>{v}</v></c>'
    return f'<c r="{ref}" t="inlineStr"><is><t>{escape(v)}</t></is></c>'

def hoja(filas):
    cols = "ABCDEFGHIJK"
    xml = ['<?xml version="1.0" encoding="UTF-8" standalone="yes"?>',
           '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>']
    for i, f in enumerate(filas, 1):
        xml.append(f'<row r="{i}">' + "".join(celda(f"{cols[j]}{i}", v) for j, v in enumerate(f)) + "</row>")
    xml.append("</sheetData></worksheet>")
    return "".join(xml)

def escribir(ruta, filas):
    with zipfile.ZipFile(ruta, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>')
        z.writestr("_rels/.rels", '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>')
        z.writestr("xl/workbook.xml", '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Bienes privados" sheetId="1" r:id="rId1"/></sheets></workbook>')
        z.writestr("xl/_rels/workbook.xml.rels", '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>')
        z.writestr("xl/worksheets/sheet1.xml", hoja(filas))

enc = ["agrupacion", "numero", "tipo", "coeficiente", "propietario", "documento", "porcentaje"]
destino = sys.argv[1]
escribir(destino + "/bienes_privados.xlsx", [
    enc,
    ["3", 506, "apartamento", 0.00523, "Ana Pérez", "52123456", ""],
    ["3", 507, "apartamento", 0.99477, "Luis Gómez", 79123456, ""],
])
escribir(destino + "/bienes_privados_con_errores.xlsx", [
    enc,
    ["3", 506, "apartamento", 0.5, "Ana Pérez", "52123456", ""],
    ["3", "", "apartamento", "dos", "", "", ""],
    ["3", 508, "castillo", 0.5, "", "", ""],
])
