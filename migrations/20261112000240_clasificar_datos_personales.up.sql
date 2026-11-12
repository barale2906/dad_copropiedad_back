-- Ley 1581 de 2012 (ADR 0004): columnas que pueden contener datos personales y faltaban por
-- clasificar. Los contactos de una organización son de una persona natural cuando el proveedor
-- lo es; los textos libres los escriben personas y pueden nombrar o describir a otras.
COMMENT ON COLUMN organizaciones.correo_contacto IS 'dato_personal: en personas naturales es su correo';
COMMENT ON COLUMN organizaciones.telefono_contacto IS 'dato_personal: en personas naturales es su teléfono';
COMMENT ON COLUMN visitas.observacion IS 'dato_personal: texto libre de la portería sobre el visitante';
COMMENT ON COLUMN reservas.observacion IS 'dato_personal: texto libre sobre la reserva (puede nombrar personas)';
COMMENT ON COLUMN autorizaciones_acceso.motivo IS 'dato_personal: texto libre de quien autoriza (puede nombrar personas)';
COMMENT ON COLUMN asistente_presencias.detalle_salida IS 'dato_personal: texto libre sobre la salida del asistente';
COMMENT ON COLUMN cambios_censo.motivo IS 'dato_personal: texto libre de quien propone el cambio';
COMMENT ON COLUMN reclamaciones_organizacion.motivo IS 'dato_personal: texto libre de quien reclama la organización';
COMMENT ON COLUMN ordenes_trabajo.observaciones IS 'dato_personal: texto libre del trabajo (puede nombrar personas)';
