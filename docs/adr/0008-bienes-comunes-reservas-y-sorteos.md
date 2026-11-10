# ADR 0008 · Bienes comunes, motor de reservas, parqueaderos y sorteos verificables

- **Estado:** aceptada
- **Fecha:** 2026-11-07

## Contexto
Cada PH define sus bienes comunes (salones, piscinas, gimnasios…), sus reglas de uso, horarios,
tarifas y forma de asignación; administra parqueaderos privados, comunales y de visitantes, y
atiende citas. Es un módulo vendible (`bienes_comunes`).

## Decisión
- **Un solo motor**: `recursos` (bien común, parqueadero, agenda de la administración,
  herramienta) con su **política de reserva** como columnas (`domain::PoliticaReserva`): modo de
  asignación, duración, granularidad, anticipación, límites por bien privado, alistamiento,
  quién reserva, aforo, tarifas (ordinaria y festivo; por reserva o por hora), depósito,
  penalidad por cancelación tardía, invitados y método de entrega. Las citas son reservas de la
  agenda con modalidad y motivo; los parqueaderos de visitantes, reservas con placa.
- **Horarios en la hora local de la PH** (`copropiedades.zona_horaria`, zona IANA con
  `chrono-tz`), con festivos de Colombia y excepciones por fecha (`domain::Horario`).
- **Sin solapes en la BD**: `EXCLUDE USING gist (recurso_id WITH =, franja WITH &&)` sobre las
  reservas activas de recursos exclusivos (extensión `btree_gist`). Para recursos con **aforo**
  se bloquea el recurso (`FOR UPDATE`) y se cuentan las personas de la franja. Solicitar es
  idempotente (`UNIQUE (tenant_id, idempotency_key)`).
- **Cobro**: se liquida y registra el valor; el estado del pago (`pendiente`, `pagado`,
  `exonerado`) se marca a mano. El pago en línea queda para una pasarela o para el módulo de
  contabilidad (decisión del usuario: se admitirán ambos). Penalidades y daños quedan como
  `cargos_pendientes` del bien privado.
- **Restricción por mora o sanción desde ya**: marca manual (`restricciones_reserva`, con
  soporte y vigencia). No aplica a los bienes comunes esenciales (art. 59).
- **Tenedores**: la política dice si necesitan autorización del propietario para reservar
  (`tenedor_requiere_autorizacion`); el propietario o la administración la otorgan en la
  residencia (`reservas_autorizadas`). Cada PH escoge según su manual de convivencia.
- **Parqueaderos**: inventario con naturaleza (privado, uso exclusivo, comunal, visitantes) y
  asignaciones con vigencia sin solapes (`EXCLUDE` sobre `tstzrange`).
- **Sorteos verificables** (compromiso–revelación): al crear el sorteo se publica el SHA-256 de
  una semilla aleatoria; al ejecutarlo se revela y cualquiera reproduce el orden con
  `domain::sorteo::ejecutar`. Orden por **puntaje** (discapacidad, no favorecido en el sorteo
  anterior) y luego pseudoaleatorio por semilla. El puntaje es extensible: más adelante se
  podrán sumar o restar factores (rotación, reincidencia, llamados de atención). La periodicidad
  la define cada sorteo con su vigencia.
- **Recordatorios**: el worker del gateway encola cada 5 minutos el recordatorio (24 h antes)
  de las reservas aprobadas que aún no lo tienen (marca `recordatorio_enviado_en`); una reserva
  cancelada antes nunca se recuerda.

## Consecuencias
- La semilla se guarda en la BD desde la creación del sorteo; la API no la muestra antes de
  ejecutarlo. Un administrador de la base podría leerla: riesgo aceptado (la auditoría registra
  el compromiso al crear y la semilla al ejecutar).
- Las novedades del recibo generan cargo pendiente y, si la PH tiene la gestión administrativa,
  un ticket de mantenimiento ligado al recurso, idempotente por reserva y a nombre de quien
  registró el recibo (plan 10, fase 3). Si no se puede radicar, el recibo igual queda.
- `chrono-tz` y la extensión `btree_gist` se incorporan al proyecto.
