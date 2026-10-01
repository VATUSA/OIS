#!/usr/bin/env bash
# Generate the self-contained, paste-able Portainer variant of the observability stack (#391) from
# the canonical config sources in this directory. The sources here stay the editable source of
# truth (and still drive the local `-f` merge stack in docker-compose.observability.yml); this
# script inlines them into `docker-compose.observability.portainer.yml` as Docker `configs`, so
# that file needs no bind mounts, no repo, and no files on the host — you paste it into Portainer.
#
# Re-run after editing any source file:  deploy/observability/gen-portainer-compose.sh
# CI (or a reviewer) can diff the result to catch a stale generated file.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$root/docker-compose.observability.portainer.yml"

# Emit a file's body indented under a `content: |` block (6 spaces). The dashboard provider path is
# rewritten off the Grafana data volume (/var/lib/grafana) to a plain config dir, so the inline
# config file can't collide with the ois_grafana volume mount.
emit() { sed 's#/var/lib/grafana/dashboards#/etc/grafana/dashboards#g; s/^/      /' "$1"; }

{
cat <<'YAML'
# GENERATED — do not edit by hand. Regenerate with deploy/observability/gen-portainer-compose.sh
# (source of truth is deploy/observability/*).
#
# Self-contained observability stack for a Portainer web-editor (paste-the-YAML) deployment: it
# carries its Prometheus + Grafana config INLINE as Docker configs, so there are no bind mounts and
# nothing has to exist on the host. Deploy it as its own Portainer stack, separate from the base
# OIS stack.
#
#   1. Deploy the base OIS stack (docker-compose.yml) first — it creates the shared network.
#   2. New stack -> Web editor -> paste this file. Set OIS_NETWORK to your base stack's network
#      (its name is "<base-stack-name>_default"; check Portainer -> Networks or `docker network
#      ls`). Default below is ois_default.
#   3. Reach Grafana at ${GRAFANA_PORT:-3001}; log in with GRAFANA_ADMIN_PASSWORD. The Prometheus
#      datasource and the OIS dashboard are already provisioned.
#
# Requires a Compose that supports inline `configs` content (Docker Compose v2.23.1+); Portainer
# 2.19+ ships one. /metrics is unauthenticated but only scraped in-network; see docs/deploy.md to
# gate it with METRICS_TOKEN.
services:
  prometheus:
    image: prom/prometheus:v3.7.3
    configs:
      - source: prometheus_config
        target: /etc/prometheus/prometheus.yml
    volumes:
      - ois_prometheus:/prometheus
    command:
      - --config.file=/etc/prometheus/prometheus.yml
      - --storage.tsdb.path=/prometheus
      - --storage.tsdb.retention.time=${PROMETHEUS_RETENTION:-15d}
    ports:
      - "${BIND_HOST:-127.0.0.1}:${PROMETHEUS_PORT:-9090}:9090"
    restart: unless-stopped

  grafana:
    image: grafana/grafana:12.3.1
    depends_on:
      - prometheus
    environment:
      GF_SECURITY_ADMIN_PASSWORD: ${GRAFANA_ADMIN_PASSWORD:-admin}
      GF_USERS_ALLOW_SIGN_UP: "false"
    configs:
      - source: grafana_datasource
        target: /etc/grafana/provisioning/datasources/prometheus.yml
      - source: grafana_dashboard_provider
        target: /etc/grafana/provisioning/dashboards/ois.yml
      - source: grafana_dashboard_ois
        target: /etc/grafana/dashboards/ois-overview.json
    volumes:
      - ois_grafana:/var/lib/grafana
    ports:
      - "${BIND_HOST:-127.0.0.1}:${GRAFANA_PORT:-3001}:3000"
    restart: unless-stopped

# The base OIS stack's network, joined as external so Prometheus can scrape backend:3000 in-network
# without this stack redefining any of those services.
networks:
  default:
    external: true
    name: ${OIS_NETWORK:-ois_default}

volumes:
  ois_prometheus:
  ois_grafana:

configs:
  prometheus_config:
    content: |
YAML
emit "$here/prometheus.yml"
echo "  grafana_datasource:"
echo "    content: |"
emit "$here/grafana/provisioning/datasources/prometheus.yml"
echo "  grafana_dashboard_provider:"
echo "    content: |"
emit "$here/grafana/provisioning/dashboards/ois.yml"
echo "  grafana_dashboard_ois:"
echo "    content: |"
emit "$here/grafana/dashboards/ois-overview.json"
} > "$out"

echo "wrote $out"
