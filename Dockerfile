# syntax = docker/dockerfile:1.3@sha256:42399d4635eddd7a9b8a24be879d2f9a930d0ed040a61324cfdf59ef1357b3b2

FROM docker.io/library/ubuntu:24.04@sha256:786a8b558f7be160c6c8c4a54f9a57274f3b4fb1491cf65146521ae77ff1dc54

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

# sysand-amd64 / sysand-arm64 are populated via the publish-images workflow/job
COPY --chmod=0755 sysand-${TARGETARCH}/sysand /usr/local/bin/sysand
