# syntax = docker/dockerfile:1.3

FROM docker.io/library/ubuntu:24.04

ARG TARGETARCH
ENV DEBIAN_FRONTEND=noninteractive

# ca-certificates and curl are added so that they can be used to exchange
# temporary trusted publishing tokens, relevant when using sysand publish in a
# CI context
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
 && rm -rf /var/lib/apt/lists/*

# sysand-amd64 / sysand-arm64 are populated via the publish-image workflow/job
COPY --chmod=0755 sysand-${TARGETARCH}/sysand /usr/local/bin/sysand
