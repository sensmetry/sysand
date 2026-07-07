# syntax = docker/dockerfile:1.24@sha256:87999aa3d42bdc6bea60565083ee17e86d1f3339802f543c0d03998580f9cb89

FROM docker.io/library/ubuntu:24.04@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

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
