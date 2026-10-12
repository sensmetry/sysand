# syntax = docker/dockerfile:1.27@sha256:4edf897a3ffa55b89f906fc8cc78afdb3f1834cc9c7083565e611a8a7d5fe99e

FROM docker.io/library/ubuntu:24.04@sha256:534baea6a22c03a63003dbc8dbe78fe34bc0d7e595d9a9dc9834884ff530eb55

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
