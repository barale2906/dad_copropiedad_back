#!/usr/bin/env python3
"""Genera docs/guia-frontend-endpoints.md desde apps/api/openapi.json.

Índice de consulta para el frontend: cada operación con su permiso, cuerpo, respuesta y errores
propios, agrupada por etiqueta (módulo). El contrato manda; este archivo solo lo resume.
Uso: `make guia-endpoints` (o `python3 scripts/indice_endpoints.py`).
"""

import json
import re
import sys
from pathlib import Path

RAIZ = Path(__file__).resolve().parent.parent
CONTRATO = RAIZ / "apps/api/openapi.json"
SALIDA = RAIZ / "docs/guia-frontend-endpoints.md"
METODOS = ("get", "post", "put", "patch", "delete")
# Los imponen los extractores de sesión en casi todas las rutas (guía §5).
COMUNES = {"401", "404"}
PERMISO = re.compile(r"`([a-z_]+(?:\.[a-z_]+)+)`")


def esquema(contenido):
    """Nombre del esquema de un cuerpo o respuesta (o su tipo de contenido si no es JSON)."""
    if not contenido:
        return ""
    for tipo, c in contenido.items():
        s = c.get("schema", {})
        ref = s.get("$ref") or s.get("items", {}).get("$ref")
        if ref:
            nombre = ref.rsplit("/", 1)[-1]
            return f"[{nombre}]" if s.get("type") == "array" else nombre
        if tipo != "application/json":
            return tipo
        return s.get("type", "")
    return ""


def celda(texto):
    return (texto or "").replace("|", "\\|").replace("\n", " ").strip()


def main():
    o = json.loads(CONTRATO.read_text(encoding="utf-8"))
    etiquetas = {t["name"]: t.get("description", "") for t in o.get("tags", [])}
    grupos = {nombre: [] for nombre in etiquetas}
    for ruta, ops in o["paths"].items():
        for metodo in METODOS:
            op = ops.get(metodo)
            if not op:
                continue
            tag = (op.get("tags") or ["otros"])[0]
            grupos.setdefault(tag, []).append((ruta, metodo, op))

    total = sum(len(v) for v in grupos.values())
    lineas = [
        "# Índice de endpoints para el frontend",
        "",
        "> **Generado** desde `apps/api/openapi.json` con `make guia-endpoints`; no se edita a mano.",
        "> Ante cualquier diferencia manda el contrato. Las convenciones (sesión, errores,",
        "> paginación, archivos…) están en [guia-frontend.md](guia-frontend.md).",
        "",
        f"{total} operaciones. Rutas sin el prefijo `/api/v1`. **Permiso**: el que exige la",
        "operación según su respuesta 403; en cursiva, la regla cuando no es un permiso (ser el",
        "titular, la papeleta, el enlace firmado…); vacío = basta la sesión o la credencial.",
        "**Errores propios**: además de 401 y 404, que imponen la sesión y la entidad. 🔓 = sin",
        "sesión de persona.",
        "",
        "## Módulos",
        "",
    ]
    for tag, ops in grupos.items():
        if ops:
            lineas.append(f"- [{tag}](#{tag.replace('_', '-')}) ({len(ops)}): {etiquetas.get(tag, '')}")
    for tag, ops in grupos.items():
        if not ops:
            continue
        lineas += ["", f"## {tag}", "", celda(etiquetas.get(tag, "")), ""]
        lineas += [
            "| Método | Ruta | Qué hace | Permiso | Cuerpo | Respuesta | Errores propios |",
            "|---|---|---|---|---|---|---|",
        ]
        for ruta, metodo, op in ops:
            respuestas = op.get("responses", {})
            exito = next((c for c in respuestas if c.startswith("2")), "")
            salida = esquema(respuestas.get(exito, {}).get("content")) if exito else ""
            prohibido = respuestas.get("403", {}).get("description")
            permisos = " ".join(f"`{p}`" for p in PERMISO.findall(prohibido or ""))
            if prohibido is not None and not permisos:
                # Regla que no es un permiso (titular, papeleta, enlace firmado…).
                permisos = f"_{celda(prohibido)}_"
            errores = ", ".join(
                c for c in respuestas if not c.startswith("2") and c not in COMUNES
            )
            abierta = "🔓 " if not op.get("security") else ""
            cuerpo = esquema(op.get("requestBody", {}).get("content"))
            lineas.append(
                "| {} | {}`{}` | {} | {} | {} | {} {} | {} |".format(
                    metodo.upper(),
                    abierta,
                    ruta.removeprefix("/api/v1"),
                    celda(op.get("summary")),
                    permisos,
                    f"`{cuerpo}`" if cuerpo else "",
                    exito,
                    f"`{salida}`" if salida else "",
                    errores,
                )
            )
    SALIDA.write_text("\n".join(lineas) + "\n", encoding="utf-8")
    print(f"{SALIDA.relative_to(RAIZ)}: {total} operaciones", file=sys.stderr)


if __name__ == "__main__":
    main()
