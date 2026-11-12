DELETE FROM roles_permisos
WHERE permiso IN ('plataforma.directorio.gestionar', 'directorio.ver', 'directorio.resenar');
DELETE FROM permisos
WHERE codigo IN ('plataforma.directorio.gestionar', 'directorio.ver', 'directorio.resenar');

DROP FUNCTION resenas_reportadas(UUID, INTEGER);
DROP FUNCTION fichas_directorio_copropiedad(UUID, TEXT, TEXT, BOOLEAN, TIMESTAMPTZ, UUID, INTEGER);
DROP FUNCTION anunciante_cubre(TEXT, SMALLINT, NUMERIC, NUMERIC, TEXT, NUMERIC, NUMERIC, TEXT);
DROP FUNCTION distancia_km(NUMERIC, NUMERIC, NUMERIC, NUMERIC);

DROP TABLE reportes_resena;
DROP TABLE resenas;
DROP TRIGGER ficha_sin_categoria_restringida ON ficha_categorias;
DROP FUNCTION rechazar_categoria_restringida();
DROP TABLE ficha_categorias;
DROP TABLE fichas_directorio;
DROP TABLE anunciantes;
DROP TABLE categorias_publicidad;
