-- Aviso de inicio de sesión desde un dispositivo nuevo: sesiones de una cuenta por fecha.
CREATE INDEX ix_sesiones_cuenta_creada ON sesiones (cuenta_id, creada_en);
