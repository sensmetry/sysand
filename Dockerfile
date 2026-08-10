# syntax = docker/dockerfile:1.26@sha256:ecfaec9ed6d810b56388c508f4121597bfbba70d41a6dfeee4d8cad5f295fc32

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
