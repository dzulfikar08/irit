// Two-species competitive Lotka–Volterra model, integrated with fixed-step RK4.
//   dN1/dt = r1 N1 (1 − (N1 + a12 N2) / K1)
//   dN2/dt = r2 N2 (1 − (N2 + a21 N1) / K2)
// a12 is the effect of species 2 on species 1; a21 the effect of species 1 on species 2.
// Pure functions only, so the page script and a Node check can share them.

export interface Params {
	r1: number;
	r2: number;
	K1: number;
	K2: number;
	a12: number;
	a21: number;
	n1: number;
	n2: number;
}

export type Outcome = 'one' | 'two' | 'coexist' | 'bistable' | 'boundary';

export interface Trajectory {
	t: Float64Array;
	n1: Float64Array;
	n2: Float64Array;
	tEnd: number;
}

export function deriv(p: Params, x: number, y: number): [number, number] {
	return [p.r1 * x * (1 - (x + p.a12 * y) / p.K1), p.r2 * y * (1 - (y + p.a21 * x) / p.K2)];
}

function step(p: Params, x: number, y: number, dt: number): [number, number] {
	const [a1, b1] = deriv(p, x, y);
	const [a2, b2] = deriv(p, x + (dt / 2) * a1, y + (dt / 2) * b1);
	const [a3, b3] = deriv(p, x + (dt / 2) * a2, y + (dt / 2) * b2);
	const [a4, b4] = deriv(p, x + dt * a3, y + dt * b3);
	return [
		Math.max(0, x + (dt / 6) * (a1 + 2 * a2 + 2 * a3 + a4)),
		Math.max(0, y + (dt / 6) * (b1 + 2 * b2 + 2 * b3 + b4)),
	];
}

const MAX_STEPS = 400_000;

/** Integrate from 0 to tEnd and return `samples + 1` evenly spaced points. */
export function simulate(p: Params, tEnd: number, samples = 800, maxSteps = MAX_STEPS): Trajectory {
	const maxR = Math.max(p.r1, p.r2, 1e-9);
	const dt = Math.max(tEnd / maxSteps, Math.min(0.02 / maxR, tEnd / 4000));
	const t = new Float64Array(samples + 1);
	const n1 = new Float64Array(samples + 1);
	const n2 = new Float64Array(samples + 1);
	let x = p.n1;
	let y = p.n2;
	let now = 0;
	n1[0] = x;
	n2[0] = y;
	for (let i = 1; i <= samples; i++) {
		const target = (tEnd * i) / samples;
		while (now < target - 1e-12) {
			const h = Math.min(dt, target - now);
			[x, y] = step(p, x, y, h);
			now += h;
		}
		t[i] = target;
		n1[i] = x;
		n2[i] = y;
	}
	return { t, n1, n2, tEnd };
}

/**
 * How long the model takes to settle: integrate coarsely up to a cap, then find the last time
 * the state was more than 0.2% of the larger carrying capacity away from where it ended.
 */
export function settleTime(p: Params, cap = 4000 / Math.max(Math.min(p.r1, p.r2), 1e-3)): number {
	// Coarse steps are enough to find when it settles; the drawn curve is integrated finely.
	const coarse = simulate(p, cap, 4000, 40_000);
	const last = coarse.t.length - 1;
	const fx = coarse.n1[last];
	const fy = coarse.n2[last];
	const tol = 0.002 * Math.max(p.K1, p.K2);
	let lastFar = 0;
	for (let i = 0; i <= last; i++) {
		if (Math.hypot(coarse.n1[i] - fx, coarse.n2[i] - fy) > tol) lastFar = coarse.t[i];
	}
	const floor = 10 / Math.max(p.r1, p.r2, 1e-3);
	return Math.min(cap, Math.max(floor, lastFar * 1.25));
}

/** Can species 1 grow when rare, with species 2 sitting alone at K2? (and the mirror case) */
export function invasion(p: Params) {
	const one = p.K1 - p.a12 * p.K2; // > 0: species 1 invades species 2's equilibrium
	const two = p.K2 - p.a21 * p.K1; // > 0: species 2 invades species 1's equilibrium
	return { one, two };
}

export function classify(p: Params): Outcome {
	const { one, two } = invasion(p);
	const eps = 1e-9 * Math.max(p.K1, p.K2);
	if (Math.abs(one) <= eps || Math.abs(two) <= eps) return 'boundary';
	if (one > 0 && two > 0) return 'coexist';
	if (one > 0) return 'one';
	if (two > 0) return 'two';
	return 'bistable';
}

/** Interior equilibrium (both species present), if it lies in the positive quadrant. */
export function interior(p: Params): [number, number] | null {
	const d = 1 - p.a12 * p.a21;
	if (Math.abs(d) < 1e-12) return null;
	const x = (p.K1 - p.a12 * p.K2) / d;
	const y = (p.K2 - p.a21 * p.K1) / d;
	return x > 0 && y > 0 ? [x, y] : null;
}

export interface Preset {
	id: string;
	label: string;
	species: [string, string];
	params: Params;
	timeUnit: string;
	popUnit: string;
	/** Length of the original experiment, in timeUnit, if there was one. */
	experiment?: number;
	/** Longest run to search for settling, when the default (4000 / min r) is too short. */
	settleCap?: number;
	expected: Outcome;
}

export const PRESETS: Preset[] = [
	{
		id: 'yeast',
		label: 'Gause 1932: two yeasts',
		species: ['Saccharomyces cerevisiae', 'Schizosaccharomyces kephir'],
		// r, K, a from Gause (1932) as given by Edelstein-Keshet (2005), p. 231.
		// Starting volumes are our assumption (small, equal inocula).
		params: { r1: 0.21827, r2: 0.06069, K1: 13.0, K2: 5.8, a12: 3.15, a21: 0.439, n1: 0.45, n2: 0.45 },
		timeUnit: 'h',
		popUnit: 'volume units',
		experiment: 150,
		expected: 'two',
	},
	{
		id: 'paramecium',
		label: 'Gause 1934: two Paramecium',
		species: ['Paramecium aurelia', 'Paramecium caudatum'],
		// K and b from Table XI; a = 1.64 (caudatum on aurelia), b = 0.61 (aurelia on caudatum).
		// Starting volumes are our assumption.
		params: { r1: 1.1244, r2: 0.7944, K1: 105, K2: 64, a12: 1.64, a21: 0.61, n1: 2, n2: 2 },
		timeUnit: 'days',
		popUnit: 'volume units',
		settleCap: 30000,
		expected: 'one',
	},
	{
		id: 'coexist',
		label: 'Illustrative: coexistence',
		species: ['Species 1', 'Species 2'],
		params: { r1: 0.8, r2: 0.6, K1: 100, K2: 80, a12: 0.5, a21: 0.6, n1: 5, n2: 5 },
		timeUnit: 'time units',
		popUnit: 'units',
		expected: 'coexist',
	},
	{
		id: 'bistable',
		label: 'Illustrative: who starts first',
		species: ['Species 1', 'Species 2'],
		params: { r1: 0.7, r2: 0.7, K1: 100, K2: 90, a12: 1.4, a21: 1.6, n1: 12, n2: 20 },
		timeUnit: 'time units',
		popUnit: 'units',
		expected: 'bistable',
	},
];
