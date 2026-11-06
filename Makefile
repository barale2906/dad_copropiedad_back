# =====================================================================
# DAD_COPROPIEDAD · Backend (Rust)
# Uso: `make` o `make help` lista los comandos disponibles.
# =====================================================================

# Variables de .env (solo para los comandos de Docker; Cargo y sqlx-cli leen .env por sí mismos)
-include .env
POSTGRES_USER ?= dad
POSTGRES_DB   ?= dad_copropiedad
NOMBRE        ?=

COMPOSE := docker compose
CARGO   := cargo

.DEFAULT_GOAL := help
.PHONY: help herramientas env setup \
	    up down reiniciar-bd ps logs psql redis \
	    api gateway dev \
	    check fmt fmt-check lint test test-rapido cov verificar \
	    migrar migracion revertir migraciones sqlx-prepare \
	    deny audit build release limpiar

help: ## Muestra esta ayuda
	@awk 'BEGIN {FS = ":.*##"} \
	    /^##@/ { printf "\n\033[1m%s\033[0m\n", substr($$0, 5) } \
	    /^[a-zA-Z_-]+:.*##/ { printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2 }' $(MAKEFILE_LIST)

##@ Inicio del proyecto
herramientas: ## Instala las herramientas de desarrollo (sqlx-cli, nextest, bacon, deny, audit, llvm-cov)
	@if command -v cargo-binstall >/dev/null 2>&1; then \
	    $(CARGO) binstall -y sqlx-cli cargo-nextest bacon cargo-deny cargo-audit cargo-llvm-cov; \
	else \
	    $(CARGO) install sqlx-cli --no-default-features --features rustls,postgres; \
	    $(CARGO) install --locked cargo-nextest bacon cargo-deny cargo-audit cargo-llvm-cov; \
	fi

env: ## Crea .env a partir de .env.example (no sobrescribe uno existente)
	@if [ -f .env ]; then echo ".env ya existe (no se modifica)"; \
	else cp .env.example .env && echo ".env creado desde .env.example"; fi

setup: env up migrar ## Primer arranque: .env + contenedores + migraciones
	@echo "Listo. Ejecuta 'make api' y/o 'make gateway'."

##@ Contenedores (PostgreSQL 18 + Redis 8)
up: ## Levanta los contenedores y espera a que estén sanos
	$(COMPOSE) up -d --wait

down: ## Detiene los contenedores (conserva los datos)
	$(COMPOSE) down

reiniciar-bd: ## BORRA los volúmenes y recrea la infraestructura (pide confirmación)
	@read -p "Se borrarán TODOS los datos locales de PostgreSQL y Redis. ¿Continuar? [s/N] " r; \
	[ "$$r" = "s" ] || { echo "Cancelado"; exit 1; }
	$(COMPOSE) down -v
	$(COMPOSE) up -d --wait
	sqlx migrate run

ps: ## Estado de los contenedores
	$(COMPOSE) ps

logs: ## Logs de los contenedores en vivo
	$(COMPOSE) logs -f

psql: ## Consola psql como dueño del esquema
	$(COMPOSE) exec postgres psql -U $(POSTGRES_USER) -d $(POSTGRES_DB)

redis: ## Consola redis-cli
	$(COMPOSE) exec redis redis-cli

##@ Ejecución
api: ## Ejecuta la API REST/WebSocket (:8080; migra si RUN_MIGRATIONS=true)
	$(CARGO) api

gateway: ## Ejecuta el gateway de WhatsApp (:8081)
	$(CARGO) gateway

dev: ## Revisión continua al guardar (bacon)
	bacon

##@ Calidad
check: ## Compila rápido sin generar binarios
	$(CARGO) check --workspace --all-targets

fmt: ## Formatea el código
	$(CARGO) fmt --all

fmt-check: ## Verifica el formato sin modificar archivos
	$(CARGO) fmt --all --check

lint: ## Clippy estricto (-D warnings)
	$(CARGO) lint

test: ## Todas las pruebas (las de esquema requieren 'make up')
	$(CARGO) pruebas

test-rapido: ## Solo pruebas sin base de datos (dominio y aplicación)
	$(CARGO) nextest run -p domain -p application

cov: ## Reporte de cobertura HTML (target/llvm-cov/html/index.html)
	$(CARGO) cov

verificar: fmt-check lint test ## Lo que debe pasar antes de dar una tarea por terminada

##@ Base de datos (migraciones con DATABASE_URL = dueño del esquema)
migrar: ## Aplica las migraciones pendientes
	sqlx migrate run

migracion: ## Crea una migración reversible: make migracion NOMBRE=agregar_poderes
	@[ -n "$(NOMBRE)" ] || { echo "Uso: make migracion NOMBRE=<nombre>"; exit 1; }
	sqlx migrate add -r $(NOMBRE)

revertir: ## Revierte la última migración
	sqlx migrate revert

migraciones: ## Lista migraciones aplicadas y pendientes
	sqlx migrate info

sqlx-prepare: ## Regenera .sqlx/ (consultas offline) tras cambiar query!
	$(CARGO) sqlx prepare --workspace -- --all-targets

##@ Seguridad
deny: ## Licencias, fuentes y vulnerabilidades de dependencias
	$(CARGO) deny check

audit: ## Auditoría RustSec de dependencias
	$(CARGO) audit

##@ Compilación
build: ## Compila en modo desarrollo
	$(CARGO) build --workspace

release: ## Compila binarios optimizados (target/release)
	$(CARGO) build --workspace --release

limpiar: ## Borra artefactos de compilación (target/)
	$(CARGO) clean
