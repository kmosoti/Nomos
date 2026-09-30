# The disposable Debian host of the Phase 1 plan, decision 4: systemd as
# PID 1, D-Bus, and the distribution's account and package tools, so the
# Linux suites meet what a real host has. Built and run by
# `cargo xtask debian`; the release is a build argument, 12 or 13.
ARG RELEASE=12
FROM public.ecr.aws/docker/library/debian:${RELEASE}
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends systemd systemd-sysv dbus ca-certificates \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*
STOPSIGNAL SIGRTMIN+3
CMD ["/sbin/init"]
