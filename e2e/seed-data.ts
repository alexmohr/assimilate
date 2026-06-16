// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

const BASE_URL = 'http://localhost:8080';

interface LoginResponse {
  user: {
    id: number;
    username: string;
    role: string;
  };
}

interface AgentRow {
  id: number;
  hostname: string;
  display_name: string | null;
}

interface CreateAgentResponse {
  agent: AgentRow;
  token: string;
}

interface RepoRow {
  id: number;
  name: string;
}

interface ScheduleRow {
  id: number;
  repo_id: number;
}

interface UserRow {
  id: number;
  username: string;
  role: string;
}

interface CreateTokenResponse {
  id: number;
  name: string;
  token: string;
}

interface ExcludeRow {
  id: number;
  pattern: string;
}

async function apiPost<T>(
  path: string,
  body: unknown,
  cookie?: string,
): Promise<T> {
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
  };
  if (cookie !== undefined) {
    headers['Cookie'] = cookie;
  }
  const res = await fetch(`${BASE_URL}${path}`, {
    method: 'POST',
    headers,
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`POST ${path} failed (${res.status}): ${text}`);
  }
  return res.json() as Promise<T>;
}

async function login(): Promise<string> {
  const res = await fetch(`${BASE_URL}/api/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username: 'admin', password: 'admin' }),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`Login failed (${res.status}): ${text}`);
  }
  const setCookie = res.headers.get('set-cookie') ?? '';
  const match = setCookie.match(/session=([^;]+)/);
  if (match === null) {
    throw new Error('No session cookie in login response');
  }
  const _body = (await res.json()) as LoginResponse;
  return `session=${match[1]}`;
}

async function seedHosts(cookie: string): Promise<AgentRow[]> {
  const hostnames = ['web-01', 'db-01', 'mail-01'];
  const agents: AgentRow[] = [];
  for (const hostname of hostnames) {
    const resp = await apiPost<CreateAgentResponse>(
      '/api/agents',
      { hostname, display_name: hostname },
      cookie,
    );
    agents.push(resp.agent);
    process.stdout.write(`Created host: ${hostname} (id=${resp.agent.id})\n`);
  }
  return agents;
}

async function seedRepos(cookie: string): Promise<RepoRow[]> {
  const repoSpecs = [
    {
      name: 'web-01-home',
      repo_path: '/backup/repos/web-01-home',
      ssh_user: 'borg',
      ssh_host: 'localhost',
      ssh_port: 22,
      passphrase: 'devpass',
    },
    {
      name: 'db-01-data',
      repo_path: '/backup/repos/db-01-data',
      ssh_user: 'borg',
      ssh_host: 'localhost',
      ssh_port: 22,
      passphrase: 'devpass',
    },
  ];
  const repos: RepoRow[] = [];
  for (const spec of repoSpecs) {
    const repo = await apiPost<RepoRow>('/api/repos', spec, cookie);
    repos.push(repo);
    process.stdout.write(`Created repo: ${spec.name} (id=${repo.id})\n`);
  }
  return repos;
}

async function seedSchedules(
  cookie: string,
  agents: AgentRow[],
  repos: RepoRow[],
): Promise<ScheduleRow[]> {
  const scheduleSpecs = [
    {
      agent_ids: [agents[0].id],
      repo_id: repos[0].id,
      cron_expression: '0 2 * * *',
      enabled: true,
    },
    {
      agent_ids: [agents[1].id],
      repo_id: repos[1].id,
      cron_expression: '0 3 * * *',
      enabled: true,
    },
  ];
  const schedules: ScheduleRow[] = [];
  for (const spec of scheduleSpecs) {
    const schedule = await apiPost<ScheduleRow>('/api/schedules', spec, cookie);
    schedules.push(schedule);
    process.stdout.write(
      `Created schedule for agent=${spec.agent_ids[0]} repo=${spec.repo_id}\n`,
    );
  }
  return schedules;
}

async function seedUser(cookie: string): Promise<UserRow> {
  const user = await apiPost<UserRow>(
    '/api/users',
    { username: 'viewer', password: 'viewerpass1', role: 'user' },
    cookie,
  );
  process.stdout.write(`Created user: viewer (id=${user.id})\n`);
  return user;
}

async function seedToken(cookie: string): Promise<CreateTokenResponse> {
  const token = await apiPost<CreateTokenResponse>(
    '/api/tokens',
    { name: 'e2e-test-token' },
    cookie,
  );
  process.stdout.write(`Created API token: ${token.name} (id=${token.id})\n`);
  return token;
}

async function seedExcludes(cookie: string): Promise<ExcludeRow[]> {
  const patterns = ['**/.cache', '**/node_modules'];
  const excludes: ExcludeRow[] = [];
  for (const pattern of patterns) {
    const exclude = await apiPost<ExcludeRow>(
      '/api/excludes',
      { pattern },
      cookie,
    );
    excludes.push(exclude);
    process.stdout.write(`Created exclude: ${pattern} (id=${exclude.id})\n`);
  }
  return excludes;
}

async function main(): Promise<void> {
  process.stdout.write('Seeding database...\n');
  const cookie = await login();
  process.stdout.write('Logged in as admin\n');

  const agents = await seedHosts(cookie);
  const repos = await seedRepos(cookie);
  await seedSchedules(cookie, agents, repos);
  await seedUser(cookie);
  await seedToken(cookie);
  await seedExcludes(cookie);

  process.stdout.write('Seed complete.\n');
}

main().catch((err: unknown) => {
  process.stderr.write(`Seed failed: ${String(err)}\n`);
  process.exit(1);
});
