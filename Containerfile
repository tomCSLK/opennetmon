# Container image of opennetmon (docs/docker.md): the static binary on an empty base, no shell, no package manager.
#
# Built by dev/container.sh (locally) and .github/workflows/container.yml (releases) from a prepared context, not from
# the repository root. There are no RUN steps, so every architecture builds natively without emulation:
#   rootfs/  the binary, LICENSE, THIRD-PARTY-LICENSES.txt and /tmp (owned by root)
#   state/   the empty directories /etc/opennetmon and /var/lib/opennetmon (owned by 10001, the user the server runs as)
FROM scratch

ARG VERSION=dev
ARG REVISION=unknown
ARG CREATED=unknown

LABEL org.opencontainers.image.title="opennetmon" \
      org.opencontainers.image.description="Agentless SNMP network monitoring with a web interface" \
      org.opencontainers.image.url="https://github.com/tomCSLK/opennetmon" \
      org.opencontainers.image.source="https://github.com/tomCSLK/opennetmon" \
      org.opencontainers.image.documentation="https://github.com/tomCSLK/opennetmon/blob/main/docs/docker.md" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later" \
      org.opencontainers.image.authors="Thomas Cieslik" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.created="${CREATED}"

COPY rootfs/ /
COPY --chown=10001:10001 state/ /

# All settings as variables (docs/configuration.md); override them in compose.yaml or with `docker run -e`.
# Built-in HTTPS on 8443 with a self-signed certificate that the server creates on the first start, like the .deb.
ENV OPENNETMON_LISTEN=0.0.0.0:8443 \
    OPENNETMON_DATABASE_PATH=/var/lib/opennetmon/opennetmon.db \
    OPENNETMON_SECRET_KEY_FILE=/etc/opennetmon/secret.key \
    OPENNETMON_TLS_CERT_FILE=/etc/opennetmon/tls-cert.pem \
    OPENNETMON_TLS_KEY_FILE=/etc/opennetmon/tls-key.pem \
    OPENNETMON_GENERATE_MISSING=true \
    OPENNETMON_BACKUP_DIR=/var/lib/opennetmon/backups \
    OPENNETMON_UPDATE_CHECK=true \
    OPENNETMON_PACKAGING=container

USER 10001:10001
EXPOSE 8443
# The key and the certificate live apart from the database and its backups.
VOLUME ["/etc/opennetmon", "/var/lib/opennetmon"]
HEALTHCHECK --interval=30s --timeout=10s --start-period=30s --retries=3 CMD ["/usr/bin/opennetmon", "healthcheck"]
ENTRYPOINT ["/usr/bin/opennetmon"]
CMD ["serve"]
