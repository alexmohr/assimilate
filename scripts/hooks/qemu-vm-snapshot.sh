#!/bin/sh

# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

# Snapshot libvirt/QEMU domains into DEST_ROOT/<domain name> so a borg schedule
# can back them up as ordinary files. Meant to be run as a pre-backup hook.
#
# Running domains whose disks are all qcow2 use libvirt's incremental backup
# API: the first run writes a full image and creates a checkpoint, every later
# run writes only the clusters that changed since the previous checkpoint.
# Domains that cannot do that (raw disks, libvirt without backup-begin) fall
# back to a full copy taken from a temporary external snapshot, and shut off
# domains are copied directly and skipped entirely while their disks are
# unchanged.
#
# Usage: qemu-vm-snapshot.sh [domain ...]
# Without arguments every defined domain is processed.
#
# Environment:
#   DEST_ROOT            target directory (default /home/virt/backups)
#   FULL_INTERVAL        write a new full image after this many increments (7)
#   JOB_TIMEOUT          seconds to wait for one backup job (1800)
#   SKIP_DOMAINS         space separated domain names to leave out
#   MAX_SIZE             storage one domain may use in DEST_ROOT, for example
#                        200G (0, the default, means no limit)
#   MAX_SIZE_OVERRIDES   space separated domain=size pairs overriding MAX_SIZE
#   TARGET_OWNER         chown the target directories to this user (optional,
#                        needed when qemu runs as a non-root user)
#   LIBVIRT_DEFAULT_URI  libvirt connection (default qemu:///system)

set -eu

DEST_ROOT="${DEST_ROOT:-/home/virt/backups}"
FULL_INTERVAL="${FULL_INTERVAL:-7}"
JOB_TIMEOUT="${JOB_TIMEOUT:-1800}"
SKIP_DOMAINS="${SKIP_DOMAINS:-}"
MAX_SIZE="${MAX_SIZE:-0}"
MAX_SIZE_OVERRIDES="${MAX_SIZE_OVERRIDES:-}"
TARGET_OWNER="${TARGET_OWNER:-}"
LIBVIRT_DEFAULT_URI="${LIBVIRT_DEFAULT_URI:-qemu:///system}"
export LIBVIRT_DEFAULT_URI

CHECKPOINT_PREFIX="assimilate"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
FAILED=""

log() {
    printf '%s qemu-vm-snapshot: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*"
}

fail() {
    log "ERROR: $*" >&2
}

is_skipped() {
    for skip in $SKIP_DOMAINS; do
        [ "$skip" = "$1" ] && return 0
    done
    return 1
}

# Converts a size such as 200G into KiB. Fails on anything unparsable, prints
# 0 for an empty value, which means "no limit".
parse_size() {
    size_value="$1"
    [ -n "$size_value" ] || {
        printf '0'
        return 0
    }
    size_number="${size_value%[KkMmGgTt]}"
    size_suffix="${size_value#"$size_number"}"
    case "$size_number" in
        "" | *[!0-9]*) return 1 ;;
    esac
    case "$size_suffix" in
        K | k) printf '%s' "$size_number" ;;
        M | m) printf '%s' "$((size_number * 1024))" ;;
        G | g) printf '%s' "$((size_number * 1024 * 1024))" ;;
        T | t) printf '%s' "$((size_number * 1024 * 1024 * 1024))" ;;
        "") printf '%s' "$(((size_number + 1023) / 1024))" ;;
        *) return 1 ;;
    esac
}

format_kib() {
    if [ "$1" -ge 1048576 ]; then
        printf '%s GiB' "$(($1 / 1048576))"
    elif [ "$1" -ge 1024 ]; then
        printf '%s MiB' "$(($1 / 1024))"
    else
        printf '%s KiB' "$1"
    fi
}

# Prints the limit of a domain in KiB, 0 when it is unlimited.
domain_limit() {
    limit_value="$MAX_SIZE"
    for limit_entry in $MAX_SIZE_OVERRIDES; do
        case "$limit_entry" in
            "$1="*) limit_value="${limit_entry#*=}" ;;
        esac
    done
    parse_size "$limit_value"
}

# Disk space in KiB, as allocated on disk rather than as apparent size.
path_usage_kib() {
    if [ -b "$1" ] && command -v blockdev >/dev/null 2>&1; then
        usage_bytes="$(blockdev --getsize64 "$1" 2>/dev/null || true)"
        case "$usage_bytes" in
            "" | *[!0-9]*) ;;
            *)
                printf '%s' "$(((usage_bytes + 1023) / 1024))"
                return 0
                ;;
        esac
    fi
    usage_kib="$(du -sk "$1" 2>/dev/null | awk 'NR == 1 { print $1 }')"
    case "$usage_kib" in
        "" | *[!0-9]*) usage_kib=0 ;;
    esac
    printf '%s' "$usage_kib"
}

disks_usage_kib() {
    disks_total=0
    while IFS=' ' read -r disks_target disks_source; do
        [ -n "$disks_target" ] || continue
        disks_total=$((disks_total + $(path_usage_kib "$disks_source")))
    done <<DISKS
$1
DISKS
    printf '%s' "$disks_total"
}

# Largest increment of the current chain, used to guess the next one.
largest_increment_kib() {
    increment_max=0
    for increment_file in "$1"/*.qcow2; do
        case "$increment_file" in
            *.full.qcow2) continue ;;
        esac
        [ -f "$increment_file" ] || continue
        increment_kib="$(path_usage_kib "$increment_file")"
        if [ "$increment_kib" -gt "$increment_max" ]; then
            increment_max="$increment_kib"
        fi
    done
    printf '%s' "$increment_max"
}

# Refuses to start when a fresh full copy cannot fit into the domain's limit.
full_backup_fits() {
    fits_domain="$1"
    fits_disks="$2"
    fits_limit="$3"

    [ "$fits_limit" -gt 0 ] || return 0
    fits_needed="$(disks_usage_kib "$fits_disks")"
    if [ "$fits_needed" -gt "$fits_limit" ]; then
        fail "$fits_domain: a full backup needs about $(format_kib "$fits_needed"), which exceeds the limit of $(format_kib "$fits_limit")"
        return 1
    fi
    return 0
}

# Prints "<target> <source path>" per writable disk of a domain.
domain_disks() {
    virsh domblklist --details "$1" 2>/dev/null |
        awk '$1 != "Type" && $2 == "disk" && $4 != "-" { print $3, $4 }'
}

disk_format() {
    qemu-img info -U "$1" 2>/dev/null | sed -n 's/^file format: //p' | head -n1
}

all_disks_qcow2() {
    qcow2_result=0
    while IFS=' ' read -r qcow2_target qcow2_source; do
        [ -n "$qcow2_target" ] || continue
        [ "$(disk_format "$qcow2_source")" = "qcow2" ] || qcow2_result=1
    done <<DISKS
$1
DISKS
    return "$qcow2_result"
}

# Checkpoints this script owns, oldest first (the names sort by timestamp).
list_checkpoints() {
    virsh checkpoint-list "$1" --name 2>/dev/null |
        grep "^$CHECKPOINT_PREFIX-" |
        sort
}

drop_checkpoints() {
    while IFS= read -r checkpoint; do
        [ -n "$checkpoint" ] || continue
        virsh checkpoint-delete "$1" "$checkpoint" >/dev/null 2>&1 ||
            virsh checkpoint-delete "$1" "$checkpoint" --metadata >/dev/null 2>&1 ||
            fail "$1: could not delete checkpoint $checkpoint"
    done <<CHECKPOINTS
$(list_checkpoints "$1")
CHECKPOINTS
}

job_state() {
    virsh domjobinfo "$@" 2>/dev/null |
        sed -n 's/^Job type:[[:space:]]*\([A-Za-z]*\).*/\1/p' |
        head -n1
}

wait_for_backup() {
    wait_domain="$1"
    wait_deadline=$(($(date +%s) + JOB_TIMEOUT))
    while :; do
        wait_state="$(job_state "$wait_domain")"
        case "$wait_state" in
            "" | None) break ;;
        esac
        if [ "$(date +%s)" -ge "$wait_deadline" ]; then
            virsh domjobabort "$wait_domain" >/dev/null 2>&1 || true
            fail "$wait_domain: backup job timed out after ${JOB_TIMEOUT}s"
            return 1
        fi
        sleep 5
    done

    wait_result="$(job_state "$wait_domain" --completed)"
    if [ "$wait_result" != "Completed" ]; then
        fail "$wait_domain: backup job did not complete (${wait_result:-no job info})"
        return 1
    fi
    return 0
}

write_backup_xml() {
    backup_domain_name="$1"
    backup_disks="$2"
    backup_from="$3"
    backup_xml="$4"

    {
        printf '<domainbackup mode="push">\n'
        if [ -n "$backup_from" ]; then
            printf '  <incremental>%s</incremental>\n' "$backup_from"
        fi
        printf '  <disks>\n'
        while IFS=' ' read -r backup_target _; do
            [ -n "$backup_target" ] || continue
            if [ -n "$backup_from" ]; then
                backup_file="$DEST_ROOT/$backup_domain_name/$backup_target.$STAMP.qcow2"
            else
                backup_file="$DEST_ROOT/$backup_domain_name/$backup_target.full.qcow2"
            fi
            rm -f "$backup_file"
            printf '    <disk name="%s" backup="yes" type="file">\n' "$backup_target"
            printf '      <driver type="qcow2"/>\n'
            printf '      <target file="%s"/>\n' "$backup_file"
            printf '    </disk>\n'
        done <<DISKS
$backup_disks
DISKS
        printf '  </disks>\n'
        printf '</domainbackup>\n'
    } >"$backup_xml"
}

write_checkpoint_xml() {
    checkpoint_disks="$1"
    checkpoint_name="$2"
    checkpoint_xml="$3"

    {
        printf '<domaincheckpoint>\n'
        printf '  <name>%s</name>\n' "$checkpoint_name"
        printf '  <disks>\n'
        while IFS=' ' read -r checkpoint_target _; do
            [ -n "$checkpoint_target" ] || continue
            printf '    <disk name="%s" checkpoint="bitmap"/>\n' "$checkpoint_target"
        done <<DISKS
$checkpoint_disks
DISKS
        printf '  </disks>\n'
        printf '</domaincheckpoint>\n'
    } >"$checkpoint_xml"
}

record_chain() {
    chain_disks="$1"
    chain_dest="$2"
    chain_suffix="$3"

    while IFS=' ' read -r chain_target _; do
        [ -n "$chain_target" ] || continue
        printf '%s %s\n' "$chain_target" "$chain_target.$chain_suffix" >>"$chain_dest/chain.txt"
    done <<DISKS
$chain_disks
DISKS
}

# Incremental (or full) push backup through libvirt checkpoints.
backup_with_checkpoints() {
    inc_domain="$1"
    inc_disks="$2"
    inc_dest="$3"
    inc_limit="$4"
    inc_used="$5"

    inc_checkpoints="$(list_checkpoints "$inc_domain")"
    inc_count="$(printf '%s\n' "$inc_checkpoints" | grep -c . || true)"
    inc_from=""
    if [ "$inc_count" -ge 1 ] && [ "$inc_count" -lt "$FULL_INTERVAL" ] && [ -f "$inc_dest/chain.txt" ]; then
        inc_from="$(printf '%s\n' "$inc_checkpoints" | tail -n1)"
    fi

    if [ -n "$inc_from" ] && [ "$inc_limit" -gt 0 ]; then
        inc_estimate="$(largest_increment_kib "$inc_dest")"
        if [ "$inc_estimate" -eq 0 ]; then
            inc_estimate=$((inc_used / 10 + 1))
        fi
        if [ "$((inc_used + inc_estimate))" -gt "$inc_limit" ]; then
            log "$inc_domain: the chain is within $(format_kib "$inc_estimate") of the $(format_kib "$inc_limit") limit, writing a new full image"
            inc_from=""
        fi
    fi

    if [ -n "$inc_from" ]; then
        log "$inc_domain: incremental backup since $inc_from"
    else
        full_backup_fits "$inc_domain" "$inc_disks" "$inc_limit" || return 1
        log "$inc_domain: full backup"
        drop_checkpoints "$inc_domain"
        rm -f "$inc_dest"/*.qcow2 "$inc_dest/chain.txt"
    fi

    inc_backup_xml="$(mktemp)"
    inc_checkpoint_xml="$(mktemp)"
    write_backup_xml "$inc_domain" "$inc_disks" "$inc_from" "$inc_backup_xml"
    write_checkpoint_xml "$inc_disks" "$CHECKPOINT_PREFIX-$STAMP" "$inc_checkpoint_xml"

    # Drop stale completed job statistics so wait_for_backup reads this run.
    virsh domjobinfo "$inc_domain" --completed >/dev/null 2>&1 || true

    inc_status=0
    if ! virsh backup-begin "$inc_domain" "$inc_backup_xml" "$inc_checkpoint_xml" >/dev/null; then
        fail "$inc_domain: backup-begin failed"
        inc_status=1
    elif ! wait_for_backup "$inc_domain"; then
        inc_status=1
    fi
    rm -f "$inc_backup_xml" "$inc_checkpoint_xml"
    [ "$inc_status" -eq 0 ] || return 1

    if [ -n "$inc_from" ]; then
        record_chain "$inc_disks" "$inc_dest" "$STAMP.qcow2"
    else
        record_chain "$inc_disks" "$inc_dest" "full.qcow2"
    fi
    return 0
}

# Full copy of a running domain, taken from a temporary external snapshot.
backup_running_copy() {
    live_domain="$1"
    live_disks="$2"
    live_dest="$3"
    live_limit="$4"
    live_snapshot="$CHECKPOINT_PREFIX-tmp-$STAMP"

    full_backup_fits "$live_domain" "$live_disks" "$live_limit" || return 1
    log "$live_domain: full copy through an external snapshot"
    drop_checkpoints "$live_domain"
    rm -f "$live_dest"/*.qcow2
    if ! virsh snapshot-create-as --domain "$live_domain" --name "$live_snapshot" \
        --disk-only --atomic --no-metadata --quiesce >/dev/null 2>&1; then
        if ! virsh snapshot-create-as --domain "$live_domain" --name "$live_snapshot" \
            --disk-only --atomic --no-metadata >/dev/null; then
            fail "$live_domain: could not create an external snapshot"
            return 1
        fi
        log "$live_domain: snapshot taken without guest agent quiesce"
    fi

    live_status=0
    : >"$live_dest/chain.txt"
    while IFS=' ' read -r live_target live_source; do
        [ -n "$live_target" ] || continue
        if cp --reflink=auto --sparse=always "$live_source" "$live_dest/$live_target.img"; then
            printf '%s %s\n' "$live_target" "$live_target.img" >>"$live_dest/chain.txt"
        else
            fail "$live_domain: could not copy $live_source"
            live_status=1
        fi
    done <<DISKS
$live_disks
DISKS

    # Merge the overlay back into the base image and drop the overlay file.
    while IFS=' ' read -r live_target live_source; do
        [ -n "$live_target" ] || continue
        if virsh blockcommit "$live_domain" "$live_target" --active --pivot --wait >/dev/null; then
            rm -f "$live_source.$live_snapshot"
        else
            fail "$live_domain: blockcommit of $live_target failed, the domain still runs on the overlay $live_source.$live_snapshot"
            live_status=1
        fi
    done <<DISKS
$live_disks
DISKS

    return "$live_status"
}

# Shut off domain: copy each disk, skipping the ones that did not change.
backup_offline_copy() {
    off_domain="$1"
    off_disks="$2"
    off_dest="$3"
    off_limit="$4"

    full_backup_fits "$off_domain" "$off_disks" "$off_limit" || return 1
    drop_checkpoints "$off_domain"
    rm -f "$off_dest"/*.qcow2

    off_status=0
    : >"$off_dest/chain.txt"
    while IFS=' ' read -r off_target off_source; do
        [ -n "$off_target" ] || continue
        off_file="$off_dest/$off_target.img"
        if [ -f "$off_file" ] && [ ! "$off_source" -nt "$off_file" ]; then
            log "$off_domain: $off_target unchanged, keeping the previous copy"
        elif ! cp --reflink=auto --sparse=always "$off_source" "$off_file"; then
            fail "$off_domain: could not copy $off_source"
            off_status=1
            continue
        fi
        printf '%s %s\n' "$off_target" "$off_target.img" >>"$off_dest/chain.txt"
    done <<DISKS
$off_disks
DISKS

    return "$off_status"
}

save_definition() {
    def_domain="$1"
    def_dest="$2"

    if ! virsh dumpxml --inactive "$def_domain" >"$def_dest/domain.xml" 2>/dev/null; then
        virsh dumpxml "$def_domain" >"$def_dest/domain.xml" || return 1
    fi

    def_nvram="$(sed -n 's:.*<nvram[^>]*>\([^<]*\)</nvram>.*:\1:p' "$def_dest/domain.xml" | head -n1)"
    if [ -n "$def_nvram" ] && [ -f "$def_nvram" ]; then
        cp -f "$def_nvram" "$def_dest/nvram.fd" || return 1
    fi
    return 0
}

backup_one() {
    one_domain="$1"
    one_dest="$DEST_ROOT/$one_domain"

    mkdir -p "$one_dest" || return 1
    if [ -n "$TARGET_OWNER" ]; then
        chown "$TARGET_OWNER" "$one_dest" || return 1
    fi

    save_definition "$one_domain" "$one_dest" || {
        fail "$one_domain: could not save the domain definition"
        return 1
    }

    one_disks="$(domain_disks "$one_domain")"
    if [ -z "$one_disks" ]; then
        log "$one_domain: no writable disks, only the definition was saved"
        return 0
    fi

    one_limit="$(domain_limit "$one_domain")"
    one_used="$(path_usage_kib "$one_dest")"

    one_status=0
    one_state="$(virsh domstate "$one_domain" 2>/dev/null || true)"
    case "$one_state" in
        running | paused | pmsuspended)
            if virsh help backup-begin >/dev/null 2>&1 && all_disks_qcow2 "$one_disks"; then
                backup_with_checkpoints "$one_domain" "$one_disks" "$one_dest" \
                    "$one_limit" "$one_used" || one_status=1
            else
                backup_running_copy "$one_domain" "$one_disks" "$one_dest" \
                    "$one_limit" || one_status=1
            fi
            ;;
        "shut off" | "shutoff")
            log "$one_domain: shut off, copying the disks directly"
            backup_offline_copy "$one_domain" "$one_disks" "$one_dest" \
                "$one_limit" || one_status=1
            ;;
        *)
            fail "$one_domain: unexpected domain state '${one_state:-unknown}'"
            return 1
            ;;
    esac
    [ "$one_status" -eq 0 ] || return 1

    if [ "$one_limit" -gt 0 ]; then
        one_used="$(path_usage_kib "$one_dest")"
        if [ "$one_used" -gt "$one_limit" ]; then
            fail "$one_domain: the snapshots use $(format_kib "$one_used"), which exceeds the limit of $(format_kib "$one_limit"). Raise MAX_SIZE for this domain or lower FULL_INTERVAL."
            return 1
        fi
        log "$one_domain: $(format_kib "$one_used") of $(format_kib "$one_limit") used"
    fi
    return 0
}

main() {
    for tool in virsh qemu-img; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            fail "$tool is not installed"
            exit 1
        fi
    done

    if ! parse_size "$MAX_SIZE" >/dev/null; then
        fail "MAX_SIZE is not a valid size: $MAX_SIZE"
        exit 1
    fi
    for override in $MAX_SIZE_OVERRIDES; do
        case "$override" in
            ?*=?*) ;;
            *)
                fail "MAX_SIZE_OVERRIDES entries look like domain=size, got: $override"
                exit 1
                ;;
        esac
        if ! parse_size "${override#*=}" >/dev/null; then
            fail "MAX_SIZE_OVERRIDES holds an invalid size: $override"
            exit 1
        fi
    done

    umask 077
    mkdir -p "$DEST_ROOT"

    if [ "$#" -gt 0 ]; then
        domains="$(printf '%s\n' "$@")"
    else
        domains="$(virsh list --all --name)"
    fi

    while IFS= read -r domain; do
        [ -n "$domain" ] || continue
        if is_skipped "$domain"; then
            log "$domain: skipped by SKIP_DOMAINS"
            continue
        fi
        if ! backup_one "$domain"; then
            FAILED="$FAILED $domain"
        fi
    done <<DOMAINS
$domains
DOMAINS

    if [ -n "$FAILED" ]; then
        fail "domains that could not be snapshotted:$FAILED"
        exit 1
    fi

    log "all domains snapshotted into $DEST_ROOT"
}

main "$@"
