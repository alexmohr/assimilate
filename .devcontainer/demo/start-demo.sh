#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
set -e

SSH_KEY="${SSH_KEY_DIR:-/ssh-keys}/id_ed25519"
if [ ! -f "$SSH_KEY" ]; then
    mkdir -p "$(dirname "$SSH_KEY")"
    ssh-keygen -t ed25519 -f "$SSH_KEY" -N "" -C "assimilate-demo"
fi

cp "$SSH_KEY.pub" /home/borg/.ssh/authorized_keys
chmod 600 /home/borg/.ssh/authorized_keys
chown borg:borg /home/borg/.ssh/authorized_keys

if [ ! -f /etc/ssh/ssh_host_ed25519_key ]; then
    ssh-keygen -A
fi
/usr/sbin/sshd
ssh-keyscan -H localhost >> /etc/ssh/ssh_known_hosts 2>/dev/null

eval "$(ssh-agent -s)"
ssh-add "$SSH_KEY"

echo "==> Creating demo backup source directories..."
mkdir -p /var/www/html /etc/nginx/conf.d /etc/nginx/sites-enabled /var/log/nginx
mkdir -p /var/lib/postgresql/data /etc/postgresql
mkdir -p /mnt/media/photos /mnt/media/videos
echo "<html><body>Demo web app</body></html>" > /var/www/html/index.html
echo "server { listen 80; }" > /etc/nginx/conf.d/default.conf
echo "127.0.0.1 - - [04/Jun/2026:00:00:00 +0000] \"GET / HTTP/1.1\" 200 42" > /var/log/nginx/access.log
echo "-- demo database dump" > /var/lib/postgresql/data/demo.sql
dd if=/dev/urandom of=/mnt/media/photos/demo.jpg bs=1024 count=512 2>/dev/null
dd if=/dev/urandom of=/mnt/media/videos/demo.mp4 bs=1024 count=1024 2>/dev/null
chown -R borg:borg /var/www /etc/nginx /var/log/nginx /var/lib/postgresql /mnt/media 2>/dev/null || true

echo "==> Waiting for PostgreSQL..."
until PGPASSWORD=borg_demo psql -h postgres -U borg -d borg -c "SELECT 1" > /dev/null 2>&1; do
    sleep 1
done

echo "==> Starting server..."
/app/server &
SERVER_PID=$!

echo "==> Waiting for server to be ready..."
until curl -sf http://localhost:8080/api/health > /dev/null 2>&1; do
    sleep 1
done

echo "==> Seeding demo data..."
/app/seed-demo.sh
. /tmp/agent-tokens.env

echo "==> Starting 3 agents..."
BORG_SERVER_URL=http://localhost:8080 BORG_AGENT_TOKEN="$AGENT_TOKEN_1" BORG_HOSTNAME=web-server-01 /app/agent &
BORG_SERVER_URL=http://localhost:8080 BORG_AGENT_TOKEN="$AGENT_TOKEN_2" BORG_HOSTNAME=db-server-01 /app/agent &
BORG_SERVER_URL=http://localhost:8080 BORG_AGENT_TOKEN="$AGENT_TOKEN_3" BORG_HOSTNAME=media-store-01 /app/agent &

echo ""
echo "Demo ready: http://localhost:8080"
echo "Login: admin / admin"
echo ""

trap 'kill $SERVER_PID 2>/dev/null; ssh-agent -k > /dev/null 2>&1' EXIT INT TERM
wait $SERVER_PID
