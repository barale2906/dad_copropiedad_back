DELETE FROM roles_permisos WHERE permiso IN (
    'tickets.ver_todos', 'tickets.configurar', 'tickets.convivencia.ver',
    'tickets.urgencias.recibir', 'conversaciones.atender', 'preguntas_frecuentes.editar',
    'inventario.ver', 'inventario.editar', 'mantenimiento.planificar', 'mantenimiento.ejecutar',
    'mantenimiento.verificar')
   OR (rol_id = uuid_rol_plantilla('asistente_administrativo') AND permiso = 'tickets.atender');
DELETE FROM permisos WHERE codigo IN (
    'tickets.ver_todos', 'tickets.configurar', 'tickets.convivencia.ver',
    'tickets.urgencias.recibir', 'conversaciones.atender', 'preguntas_frecuentes.editar',
    'inventario.ver', 'inventario.editar', 'mantenimiento.planificar', 'mantenimiento.ejecutar',
    'mantenimiento.verificar');

DROP FUNCTION copropiedades_con_gestion_pendiente();
DROP FUNCTION linea_whatsapp_predeterminada();
DROP FUNCTION destinatarios_con_permiso(TEXT);
DROP FUNCTION vinculos_por_telefono(TEXT);

DROP TABLE cumplimientos_obligacion;
DROP TABLE obligaciones_periodicas;
DROP TABLE orden_evidencias;
DROP TABLE ordenes_trabajo;
DROP TABLE planes_mantenimiento;
DROP TABLE mensajes_entrantes;
DROP TABLE sesiones_chat;
DROP TABLE conversacion_mensajes;
DROP TABLE conversaciones;
DROP TABLE preguntas_frecuentes;
DROP TABLE ticket_eventos;
DROP TABLE ticket_adjuntos;
DROP TABLE ticket_mensajes;
DROP TABLE tickets;
DROP TABLE movimientos_inventario;
DROP TABLE activo_archivos;
DROP TABLE activos;
DROP TABLE categorias_ticket;
DROP TABLE plazos_ticket;
DROP TABLE configuracion_tickets;
DROP TABLE consecutivos;
