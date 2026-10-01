# The minimal host of alpha.2 issue #35: a Debian base image and the Cell's
# package, and nothing else. Every other package on it is one the package's
# `Depends:` pulled in, so a runtime dependency the package leaves out is
# missing here, and `cargo xtask debian-minimal` and its test notice. Built
# and run by `cargo xtask debian-minimal`, which puts the package in the
# build context as `nomos-cell.deb`; the base is a build argument, the official
# Debian image of release 12 or 13 from whichever registry mirror answers.
ARG BASE=public.ecr.aws/docker/library/debian:12
FROM ${BASE}
ENV DEBIAN_FRONTEND=noninteractive
COPY nomos-cell.deb /tmp/nomos-cell.deb
RUN apt-get update \
    && apt-get install -y --no-install-recommends /tmp/nomos-cell.deb \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/* /tmp/nomos-cell.deb
STOPSIGNAL SIGRTMIN+3
# systemd, started as PID 1 by its own path: only the package's dependency on
# it can have put it here.
CMD ["/lib/systemd/systemd"]
