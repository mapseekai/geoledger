// k6 load profile for the GeoLedger HTTP API: every iteration opens a workspace,
// saves a batch of point features, publishes it and runs bbox queries against
// the published head. Driven by scripts/load-test.sh; see docs/development.md.
import http from 'k6/http';
import { check, fail } from 'k6';
import exec from 'k6/execution';
import { Counter, Rate } from 'k6/metrics';

const base = __ENV.GL_BASE_URL || fail('GL_BASE_URL is required');
const token = __ENV.GL_TOKEN || fail('GL_TOKEN is required');
const batch = Number(__ENV.GL_LOAD_BATCH || 10);

const busy = new Rate('geoledger_busy');
const published = new Counter('geoledger_published');

export const options = {
  scenarios: {
    editors: {
      executor: 'constant-vus',
      vus: Number(__ENV.GL_LOAD_VUS || 20),
      duration: __ENV.GL_LOAD_DURATION || '60s',
    },
  },
  thresholds: {
    http_req_failed: ['rate<0.01'],
    checks: ['rate>0.99'],
    geoledger_busy: ['rate<0.05'],
    'http_req_duration{op:features}': [`p(95)<${__ENV.GL_LOAD_P95_READ_MS || 500}`],
    'http_req_duration{op:save}': [`p(95)<${__ENV.GL_LOAD_P95_WRITE_MS || 1000}`],
    'http_req_duration{op:publish}': [`p(95)<${__ENV.GL_LOAD_P95_WRITE_MS || 1000}`],
  },
  summaryTrendStats: ['avg', 'p(50)', 'p(95)', 'p(99)', 'max'],
};

// 429 is the documented back-pressure answer; it is tracked by geoledger_busy
// rather than counted as a failed request.
http.setResponseCallback(http.expectedStatuses({ min: 200, max: 299 }, 429));

function uuid() {
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    return (c === 'x' ? r : (r & 0x3) | 0x8).toString(16);
  });
}

function call(op, body) {
  const response = http.post(`${base}/api/v1/${op}`, JSON.stringify(body), {
    headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    tags: { op },
  });
  busy.add(response.status === 429);
  return response;
}

export function setup() {
  const project = call('create_project', { name: `load-${Date.now()}` }).json('project');
  const dataset = call('create_dataset', { project, name: 'points', geometry_type: 'point' }).json('dataset');
  if (!project || !dataset) fail('could not create the load-test project');
  return { project, dataset };
}

export default function ({ project, dataset }) {
  const vu = exec.vu.idInTest;
  const iteration = exec.vu.iterationInScenario;
  const created = call('create_workspace', { project });
  if (created.status === 429) return;
  if (!check(created, { 'create_workspace 200': (r) => r.status === 200 })) return;
  const workspace = created.json('workspace');

  const edits = [];
  for (let k = 0; k < batch; k++) {
    const id = `vu${vu}-i${iteration}-f${k}`;
    edits.push({
      dataset,
      feature_id: id,
      feature: {
        type: 'Feature',
        id,
        properties: { vu, iteration, k, exact: '9007199254740993' },
        geometry: { type: 'Point', coordinates: [Math.random() * 20 - 10, Math.random() * 20 - 10] },
      },
    });
  }
  const saved = call('save', { project, workspace, expected_workspace_version: 0, edits });
  if (!check(saved, { 'save 200': (r) => r.status === 200 })) return;

  const request = { project, workspace, expected_workspace_version: 1, request_id: uuid(), message: `load ${vu}/${iteration}` };
  let receipt = call('publish', request);
  // A busy answer may hide a committed publication: retry the same request ID.
  for (let attempt = 0; receipt.status === 429 && attempt < 5; attempt++) receipt = call('publish', request);
  if (check(receipt, { 'publish 200': (r) => r.status === 200 })) published.add(1);

  const x = Math.random() * 18 - 10;
  const y = Math.random() * 18 - 10;
  const page = call('features', { project, dataset, bbox: [x, y, x + 2, y + 2], limit: 100 });
  check(page, { 'features 200': (r) => r.status === 200 || r.status === 429 });
}
