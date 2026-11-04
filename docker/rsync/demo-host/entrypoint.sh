#!/bin/sh
# Demo SSH host. ROLE=source seeds sample data; ROLE=target starts empty.
# Login: user "syncuser", by key (/keys/<role>_ed25519, created on first start) or by password.
set -eu
ROLE="${ROLE:-source}"
USER_NAME=syncuser
HOME_DIR=/home/$USER_NAME

# Host keys: unique per container (the image ships none), kept until the container is recreated.
ssh-keygen -A >/dev/null

# Password login (demo only).
echo "$USER_NAME:${DEMO_PASSWORD:-sync-demo}" | chpasswd

# Client key for this host, kept on the bind mount so you can paste it into the UI.
mkdir -p /keys
KEY="/keys/${ROLE}_ed25519"
if [ ! -f "$KEY" ]; then
  ssh-keygen -q -t ed25519 -N "" -C "cdc-rsync-demo-$ROLE" -f "$KEY"
fi
chmod 0644 "$KEY" "$KEY.pub"   # demo convenience: readable from the host checkout
install -d -m 0700 -o $USER_NAME -g $USER_NAME "$HOME_DIR/.ssh"
install -m 0600 -o $USER_NAME -g $USER_NAME "$KEY.pub" "$HOME_DIR/.ssh/authorized_keys"

cat > /etc/ssh/sshd_config.d/demo.conf <<CONF
PasswordAuthentication yes
PubkeyAuthentication yes
AllowAgentForwarding yes
PermitRootLogin no
CONF

if [ "$ROLE" = "source" ] && [ ! -e "$HOME_DIR/data/.seeded" ]; then
  D="$HOME_DIR/data"
  mkdir -p "$D/docs/reports" "$D/images" "$D/cache" "$D/dir with spaces"
  echo "hello from $(hostname)" > "$D/README.txt"
  for i in 1 2 3 4 5; do echo "report $i $(date -u +%FT%TZ)" > "$D/docs/reports/report-$i.csv"; done
  echo "spaces and 'quotes' & stuff" > "$D/dir with spaces/file \$name (1).txt"
  head -c 5242880 /dev/urandom > "$D/images/blob-5mb.bin"
  head -c 1048576 /dev/urandom > "$D/cache/session.tmp"
  ln -sf ../README.txt "$D/docs/readme-link"
  chmod 0640 "$D/docs/reports/report-1.csv"
  touch "$D/.seeded"
  chown -R $USER_NAME:$USER_NAME "$D"
fi

echo "demo $ROLE host ready: ssh -i /keys/${ROLE}_ed25519 $USER_NAME@$(hostname)"
exec /usr/sbin/sshd -D -e
