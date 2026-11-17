<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import BaseDisclosure from './BaseDisclosure.vue'
import CommandBlock from './CommandBlock.vue'
import EnrollmentTokenList from './EnrollmentTokenList.vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import type { InstallTarget } from '../types/enrollment'

// MOCK: the token, key and expiry are placeholders until the enrollment API exists.
defineProps<{ generated: boolean }>()

const target = ref<InstallTarget>('linux')
const expiry = ref('3600')
const hostname = ref('')
const tags = ref('')

const targets: readonly SegmentedOption<InstallTarget>[] = [
  { value: 'linux', label: 'Linux' },
  { value: 'windows', label: 'Windows' },
]

const SERVER = 'https://backup.example.com'
const TOKEN = 'enr_Zk3qT0mV9xLw2bYhN6cD8sAeJ4uPfR9w'
const SERVER_KEY = '7Qm1xB0tV3kLhYw8N2dCzEoPaU5sRjGf4HiMnXpTcA0='

const command = computed(() =>
  target.value === 'linux'
    ? `curl -fsSL ${SERVER}/install.sh | sudo sh -s -- \\\n  --token ${TOKEN} \\\n  --server-key ${SERVER_KEY}`
    : `& ([scriptblock]::Create((irm ${SERVER}/install.ps1))) \`\n  -Token ${TOKEN} \`\n  -ServerKey ${SERVER_KEY}`,
)

interface ManualStep {
  label: string
  text: string
}

const SHA256 = '9f2c41d7b0a35e6c8d1f7a94e0b3c5d268a7f1e4b9c0d3a5e7f2b8c4d6a10e93'

// MOCK: the download endpoint and checksum are placeholders.
const manualSteps = computed<ManualStep[]>(() =>
  target.value === 'linux'
    ? [
        {
          label: '1. Download and verify',
          text: `sudo curl -fsSL -o /usr/local/bin/assimilate-agent \\\n  "${SERVER}/download/agent?os=linux&arch=$(uname -m)"\necho "${SHA256}  /usr/local/bin/assimilate-agent" | sha256sum -c -\nsudo chmod 755 /usr/local/bin/assimilate-agent`,
        },
        {
          label: '2. Store the token where only root can read it',
          text: `sudo install -m 600 /dev/null /etc/assimilate-agent.env\necho 'BORG_ENROLLMENT_TOKEN=${TOKEN}' | sudo tee /etc/assimilate-agent.env >/dev/null`,
        },
        {
          label: '3. Start on boot with systemd',
          text: `sudo tee /etc/systemd/system/assimilate-agent.service >/dev/null <<'EOF'\n[Unit]\nDescription=Assimilate backup agent\nAfter=network.target\n\n[Service]\nEnvironment=BORG_SERVER_URL=${SERVER}\nEnvironment=BORG_SERVER_KEY=${SERVER_KEY}\nEnvironmentFile=/etc/assimilate-agent.env\nExecStart=/usr/local/bin/assimilate-agent\nRestart=on-failure\nRestartSec=10\n\n[Install]\nWantedBy=multi-user.target\nEOF\nsudo systemctl daemon-reload\nsudo systemctl enable --now assimilate-agent`,
        },
      ]
    : [
        {
          label: '1. Download and verify',
          text: `Invoke-WebRequest "${SERVER}/download/agent?os=windows&arch=x86_64" -OutFile agent.zip\nif ((Get-FileHash agent.zip).Hash -ne "${SHA256.toUpperCase()}") { throw "Checksum mismatch" }\nExpand-Archive agent.zip "C:\\Program Files\\Assimilate"`,
        },
        {
          label: '2. Install the service, which starts at boot',
          text: `$env:BORG_SERVER_URL = "${SERVER}"\n$env:BORG_ENROLLMENT_TOKEN = "${TOKEN}"\n$env:BORG_SERVER_KEY = "${SERVER_KEY}"\n& "C:\\Program Files\\Assimilate\\assimilate-agent\\assimilate-agent.exe" service install`,
        },
      ],
)

const autostartNote = computed(() =>
  target.value === 'linux'
    ? 'systemd starts the agent at boot and restarts it if it stops. Check it with systemctl status assimilate-agent.'
    : 'service install registers the assimilate-agent Windows service to start automatically at boot as LocalSystem. Manage it in Services (services.msc).',
)

const steps = [
  'Detects the OS and architecture, downloads the agent and verifies its SHA-256.',
  'Installs the service and writes the config with 0600 permissions.',
  'Pins the server key from the command, then enrolls and starts the agent.',
]
</script>

<template>
  <div
    v-if="!generated"
    class="form-stack"
  >
    <p class="field-hint install-intro">
      Run one command on the host. It downloads the agent, installs the service and enrolls itself.
      Use this when the server cannot reach the host over SSH.
    </p>
    <div class="field">
      <label
        class="field-label"
        for="enroll-expiry"
        >Expires after</label
      >
      <select
        id="enroll-expiry"
        v-model="expiry"
        class="input"
      >
        <option value="900">15 minutes</option>
        <option value="3600">1 hour</option>
        <option value="86400">24 hours</option>
      </select>
    </div>
    <div class="field">
      <label
        class="field-label"
        for="enroll-hostname"
        >Restrict to hostname</label
      >
      <input
        id="enroll-hostname"
        v-model="hostname"
        class="input"
        placeholder="Optional, e.g. workstation-01"
      />
      <span class="field-hint">Only a host with this exact hostname can use the token.</span>
    </div>
    <div class="field">
      <label
        class="field-label"
        for="enroll-tags"
        >Tags applied on enrollment</label
      >
      <input
        id="enroll-tags"
        v-model="tags"
        class="input"
        placeholder="Optional, e.g. servers, eu"
      />
    </div>
    <EnrollmentTokenList />
  </div>

  <div
    v-else
    class="token-notice"
  >
    <div class="install-os">
      <BaseSegmented
        v-model="target"
        :options="targets"
        label="Operating system"
      />
    </div>
    <p class="token-warning">
      Copy this command now. The token is single-use and is not shown again.
    </p>
    <CommandBlock
      :text="command"
      test-id="install-command"
    />
    <div class="install-meta">
      <span class="badge badge--warning">Expires in 59:41</span>
      <span class="badge badge--neutral">{{ hostname.trim() || 'Any hostname' }}</span>
      <span
        v-if="tags.trim()"
        class="badge badge--neutral"
        >Tags: {{ tags.trim() }}</span
      >
    </div>
    <ol class="field-hint install-steps">
      <li
        v-for="step in steps"
        :key="step"
      >
        {{ step }}
      </li>
    </ol>
    <BaseDisclosure title="Install manually">
      <div class="install-manual">
        <p class="field-hint">
          Use these steps instead of the one-line command, for example on a host that cannot pipe a
          script into a shell.
        </p>
        <CommandBlock
          v-for="step in manualSteps"
          :key="step.label"
          :label="step.label"
          :text="step.text"
        />
        <p class="field-hint">{{ autostartNote }}</p>
      </div>
    </BaseDisclosure>
  </div>
</template>

<style scoped>
.install-meta {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-4);
  margin-block: var(--space-5);
}

.install-intro {
  margin-top: var(--space-5);
}

.install-manual {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
  padding: var(--space-5);
}

.install-os {
  margin-block: var(--space-5);
}

.install-steps {
  margin: 0;
  padding-left: var(--space-8);
  list-style: decimal;
}
</style>
