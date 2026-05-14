# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        openssh-server \
        borgbackup \
        && rm -rf /var/lib/apt/lists/* \
    && mkdir /run/sshd \
    && useradd -m -s /bin/bash borg \
    && mkdir -p /home/borg/.ssh /backup/repos \
    && chmod 700 /home/borg/.ssh \
    && chown -R borg:borg /home/borg /backup

COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

EXPOSE 22

ENTRYPOINT ["/entrypoint.sh"]
