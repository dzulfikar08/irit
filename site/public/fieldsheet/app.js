// fieldsheet: offline worksheet for sample metadata. No build step, no dependencies.
// Data: IndexedDB database "fieldsheet" (stores "projects" and "rows"), on this device only.
'use strict';
(() => {
	// ---------- text (English, Bahasa Indonesia) ----------
	const STR = {
		en: {
			projects: 'Projects', noProjects: 'No projects yet. A project holds one list of samples, for example one survey or one sampling trip.',
			newProject: 'New project', projName: 'Project name', template: 'Template', create: 'Create',
			tplWater: 'Water sample (metagenome)', tplIsolate: 'Isolate',
			pattern: 'Sample ID pattern',
			patternHint: 'Used when the sample ID is left empty. {site}, {point} or any field key: that field\'s value. {rep}: A, B, C… (next free letter, for replicates). {n}, {nn}, {nnn}: row number. {date}: YYYYMMDD.',
			nSamples: '%s samples', addSample: '+ Add sample', settings: 'Project settings',
			exportCsv: 'Export CSV', shareCsv: 'Share CSV', backupProject: 'Back up this project (JSON)', backupAll: 'Back up all projects (JSON)',
			withPhotos: 'Include photos in backups', importBackup: 'Import a backup (JSON)',
			imported: 'Imported %s projects: %s new rows, %s updated, %s unchanged.', badBackup: 'This file is not a fieldsheet backup.',
			localNote: 'Data is stored only in this browser on this device. Nothing is uploaded. Use Export or Back up to move it.',
			persisted: 'The browser keeps this data until you delete it (persistent storage granted).',
			notPersisted: 'The browser may clear this data when the device is short of space. Back up often; installing the app (Add to Home Screen) makes this less likely.',
			save: 'Save', saveNext: 'Save and add next', duplicate: 'Duplicate (replicate)', delete: 'Delete',
			saved: 'Saved %s', deleted: 'Deleted %s',
			confirmDeleteRow: 'Delete sample %s? This cannot be undone.',
			confirmDeleteProject: 'Delete project "%s" and its %s samples? This cannot be undone. Back up first if unsure.',
			deleteProject: 'Delete project', required: 'Required', notNumber: 'Not a number', range: 'Must be from %s to %s',
			numList: 'Numbers separated by ; (for example 0.22; 0.45)', dupId: 'Same sample ID as another row',
			badChars: 'Use only letters, digits and . _ - (other characters become _ in sample sheets)',
			fixErrors: 'Fix these before saving:', problems: '%s with problems', dups: '%s with a duplicate ID',
			noId: '(no ID)', autoId: 'Leave empty to fill from the pattern %s', tokenMissing: 'Fill in %s to make the ID',
			gpsGet: 'Get location', gpsExact: 'Exact', gpsCoarse: 'Coarse (about 1 km)', gpsOff: 'Don\'t record',
			gpsWait: 'Getting location…', gpsFail: 'Location not available: %s', clear: 'Clear', gpsMode: 'Location precision',
			photoAdd: 'Take or choose a photo', photoRemove: 'Remove photo', photoNote: 'Kept on this device only; not in the CSV.',
			customFields: 'Custom fields', addField: 'Add field', fieldLabel: 'Field name', fieldType: 'Type',
			tText: 'Text', tNum: 'Number', tDate: 'Date', tSel: 'Choice list', fieldOptions: 'Choices, separated by ;',
			min: 'Minimum', max: 'Maximum', noCustom: 'No custom fields yet.', remove: 'Remove',
			removeFieldQ: 'Remove the field "%s"? Values already entered stay in backups but are left out of the CSV.',
			keyExists: 'A field with this name already exists', draftRestored: 'Unsaved entry restored.',
			dupWarnExport: 'Some sample IDs are used more than once (%s). Export anyway?',
			noRows: 'No samples yet.', downloaded: 'Downloaded %s', back: 'Back', langName: 'Bahasa Indonesia',
			noIdb: 'This browser cannot store data here (storage is blocked, as in some private modes).',
			prefilled: 'Prefilled from the last sample: check every value.', copyOf: 'Copy of %s: the ID will get the next letter.',
			csvCols: 'CSV columns', settingsSaved: 'Settings saved', csvNote: 'The CSV has one row per sample, with sample as the first column.',
		},
		id: {
			projects: 'Proyek', noProjects: 'Belum ada proyek. Satu proyek berisi satu daftar sampel, misalnya satu survei atau satu perjalanan sampling.',
			newProject: 'Proyek baru', projName: 'Nama proyek', template: 'Templat', create: 'Buat',
			tplWater: 'Sampel air (metagenom)', tplIsolate: 'Isolat',
			pattern: 'Pola ID sampel',
			patternHint: 'Dipakai bila ID sampel dikosongkan. {site}, {point} atau kunci kolom lain: isi kolom itu. {rep}: A, B, C… (huruf berikutnya yang belum dipakai, untuk ulangan). {n}, {nn}, {nnn}: nomor baris. {date}: YYYYMMDD.',
			nSamples: '%s sampel', addSample: '+ Tambah sampel', settings: 'Pengaturan proyek',
			exportCsv: 'Ekspor CSV', shareCsv: 'Bagikan CSV', backupProject: 'Cadangkan proyek ini (JSON)', backupAll: 'Cadangkan semua proyek (JSON)',
			withPhotos: 'Sertakan foto dalam cadangan', importBackup: 'Impor cadangan (JSON)',
			imported: '%s proyek diimpor: %s baris baru, %s diperbarui, %s tidak berubah.', badBackup: 'Berkas ini bukan cadangan fieldsheet.',
			localNote: 'Data hanya tersimpan di peramban ini, di perangkat ini. Tidak ada yang diunggah. Pakai Ekspor atau Cadangkan untuk memindahkannya.',
			persisted: 'Peramban menyimpan data ini sampai Anda menghapusnya (penyimpanan permanen diizinkan).',
			notPersisted: 'Peramban bisa menghapus data ini bila ruang perangkat hampir penuh. Sering-seringlah mencadangkan; memasang aplikasi (Tambahkan ke Layar Utama) mengurangi risikonya.',
			save: 'Simpan', saveNext: 'Simpan dan tambah lagi', duplicate: 'Duplikat (ulangan)', delete: 'Hapus',
			saved: '%s tersimpan', deleted: '%s dihapus',
			confirmDeleteRow: 'Hapus sampel %s? Tindakan ini tidak bisa dibatalkan.',
			confirmDeleteProject: 'Hapus proyek "%s" beserta %s sampelnya? Tindakan ini tidak bisa dibatalkan. Cadangkan dulu bila ragu.',
			deleteProject: 'Hapus proyek', required: 'Wajib diisi', notNumber: 'Bukan angka', range: 'Harus dari %s sampai %s',
			numList: 'Angka dipisah ; (misalnya 0,22; 0,45)', dupId: 'ID sampel sama dengan baris lain',
			badChars: 'Pakai hanya huruf, angka, dan . _ - (karakter lain menjadi _ di sample sheet)',
			fixErrors: 'Perbaiki dulu sebelum menyimpan:', problems: '%s bermasalah', dups: '%s dengan ID ganda',
			noId: '(tanpa ID)', autoId: 'Kosongkan untuk diisi dari pola %s', tokenMissing: 'Isi %s untuk membuat ID',
			gpsGet: 'Ambil lokasi', gpsExact: 'Tepat', gpsCoarse: 'Kasar (sekitar 1 km)', gpsOff: 'Jangan dicatat',
			gpsWait: 'Mengambil lokasi…', gpsFail: 'Lokasi tidak tersedia: %s', clear: 'Hapus', gpsMode: 'Ketelitian lokasi',
			photoAdd: 'Ambil atau pilih foto', photoRemove: 'Hapus foto', photoNote: 'Hanya disimpan di perangkat ini; tidak masuk CSV.',
			customFields: 'Kolom tambahan', addField: 'Tambah kolom', fieldLabel: 'Nama kolom', fieldType: 'Jenis',
			tText: 'Teks', tNum: 'Angka', tDate: 'Tanggal', tSel: 'Daftar pilihan', fieldOptions: 'Pilihan, dipisah ;',
			min: 'Minimum', max: 'Maksimum', noCustom: 'Belum ada kolom tambahan.', remove: 'Hapus',
			removeFieldQ: 'Hapus kolom "%s"? Isian yang sudah ada tetap ada di cadangan tetapi tidak lagi masuk CSV.',
			keyExists: 'Sudah ada kolom dengan nama ini', draftRestored: 'Isian yang belum disimpan dipulihkan.',
			dupWarnExport: 'Beberapa ID sampel dipakai lebih dari sekali (%s). Tetap ekspor?',
			noRows: 'Belum ada sampel.', downloaded: 'Diunduh: %s', back: 'Kembali', langName: 'English',
			noIdb: 'Peramban ini tidak bisa menyimpan data di sini (penyimpanan diblokir, seperti di sebagian mode privat).',
			prefilled: 'Diisi dari sampel terakhir: periksa setiap nilai.', copyOf: 'Salinan %s: ID akan mendapat huruf berikutnya.',
			csvCols: 'Kolom CSV', settingsSaved: 'Pengaturan tersimpan', csvNote: 'CSV berisi satu baris per sampel, dengan sample sebagai kolom pertama.',
		},
	};
	let lang = localStorage.getItem('fieldsheet-lang') || ((navigator.language || '').toLowerCase().startsWith('id') ? 'id' : 'en');
	const t = (key, ...args) => {
		let i = 0;
		return (STR[lang][key] ?? key).replace(/%s/g, () => String(args[i++] ?? ''));
	};
	const tr = (pair) => (Array.isArray(pair) ? pair[lang === 'id' ? 1 : 0] : pair || '');

	// ---------- templates ----------
	// Field: k key (CSV column), t type, l [en, id] label, h hint, req, min, max, opts [[value, en, id]].
	const POINTS = [
		['S1', 'Source (before treatment)', 'Sumber (sebelum pengolahan)'],
		['S2', 'After filter', 'Setelah filter'],
		['S3', 'Dispensed (filling tap)', 'Keluar dari kran pengisian'],
		['S4', 'Container, day 0', 'Wadah, hari ke-0'],
		['S5', 'Container, stored', 'Wadah, setelah disimpan'],
		['BF', 'Blank: field', 'Blanko: lapangan'],
		['BE', 'Blank: extraction', 'Blanko: ekstraksi'],
		['BN', 'Blank: no-template (PCR/library)', 'Blanko: tanpa templat (PCR/library)'],
	];
	const SAMPLE = { k: 'sample', t: 'text', l: ['Sample ID', 'ID sampel'], req: true };
	const NOTES = { k: 'notes', t: 'area', l: ['Notes', 'Catatan'] };
	const TEMPLATES = {
		water: {
			name: 'tplWater', pattern: '{site}-{point}-{rep}',
			fields: [
				SAMPLE,
				{ k: 'site', t: 'text', l: ['Site code', 'Kode lokasi'], h: ['For example D01', 'Misalnya D01'], req: true },
				{ k: 'point', t: 'sel', l: ['Sampling point', 'Titik sampling'], req: true, opts: POINTS },
				{ k: 'collected', t: 'dt', l: ['Date and time', 'Tanggal dan jam'], req: true, auto: true },
				{ k: 'gps', t: 'gps', l: ['Location (GPS)', 'Lokasi (GPS)'] },
				{ k: 'volume_l', t: 'num', l: ['Volume filtered (L)', 'Volume tersaring (L)'], min: 0, max: 1000 },
				{ k: 'filter_pore_um', t: 'nums', l: ['Filter pore size(s) (µm)', 'Ukuran pori filter (µm)'], min: 0, max: 100 },
				{ k: 'temp_c', t: 'num', l: ['Temperature (°C)', 'Suhu (°C)'], min: -5, max: 60 },
				{ k: 'ph', t: 'num', l: ['pH', 'pH'], min: 0, max: 14 },
				{ k: 'conductivity_us_cm', t: 'num', l: ['Conductivity (µS/cm)', 'Daya hantar listrik (µS/cm)'], min: 0, max: 200000 },
				{ k: 'tds_mg_l', t: 'num', l: ['TDS (mg/L)', 'TDS (mg/L)'], min: 0, max: 100000 },
				{ k: 'turbidity_ntu', t: 'num', l: ['Turbidity (NTU)', 'Kekeruhan (NTU)'], min: 0, max: 4000 },
				{ k: 'free_chlorine_mg_l', t: 'num', l: ['Free chlorine (mg/L)', 'Klorin bebas (mg/L)'], min: 0, max: 20 },
				NOTES,
				{ k: 'photo', t: 'photo', l: ['Photo', 'Foto'] },
			],
		},
		isolate: {
			name: 'tplIsolate', pattern: 'ISO-{nnn}',
			fields: [
				SAMPLE,
				{ k: 'source', t: 'text', l: ['Source', 'Asal'], h: ['Specimen, sample or site it came from', 'Spesimen, sampel, atau lokasi asal'] },
				{ k: 'species_guess', t: 'text', l: ['Species (guess)', 'Spesies (dugaan)'] },
				{ k: 'date', t: 'date', l: ['Date', 'Tanggal'], req: true, auto: true },
				NOTES,
			],
		},
	};
	const fieldsOf = (p) => (TEMPLATES[p.template] || TEMPLATES.isolate).fields.concat(p.custom || []);

	// ---------- small helpers ----------
	function h(tag, attrs, ...kids) {
		const el = document.createElement(tag);
		for (const [k, v] of Object.entries(attrs || {})) {
			if (v == null || v === false) continue;
			if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
			else if (k === 'class') el.className = v;
			else if (['value', 'checked', 'selected', 'disabled', 'hidden', 'required'].includes(k)) el[k] = v;
			else el.setAttribute(k, v === true ? '' : v);
		}
		for (const c of kids.flat(9)) if (c != null && c !== false) el.append(c instanceof Node ? c : String(c));
		return el;
	}
	const uid = () => (crypto.randomUUID ? crypto.randomUUID() : Date.now().toString(36) + Math.random().toString(36).slice(2));
	const pad = (n, w = 2) => String(n).padStart(w, '0');
	const nowLocal = () => {
		const d = new Date();
		return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
	};
	const today = () => nowLocal().slice(0, 10);
	const cleanName = (s) => String(s).replace(/[^A-Za-z0-9._-]/g, '_');
	const slug = (s) => String(s).toLowerCase().normalize('NFKD').replace(/[^a-z0-9]+/g, '_').replace(/^_+|_+$/g, '') || 'field';
	const num = (s) => (String(s).trim() === '' ? NaN : Number(String(s).trim().replace(',', '.')));
	const splitNums = (s) => String(s).trim().split(/[;/]|,\s+|\s+/).filter(Boolean);
	let toastTimer;
	function toast(msg) {
		const el = $('#toast');
		el.textContent = msg;
		el.classList.add('show');
		clearTimeout(toastTimer);
		toastTimer = setTimeout(() => el.classList.remove('show'), 3500);
	}
	function $(s) { return document.querySelector(s); }

	// ---------- storage (IndexedDB) ----------
	let db;
	const openDb = () =>
		new Promise((ok, fail) => {
			const req = indexedDB.open('fieldsheet', 1);
			req.onupgradeneeded = () => {
				const d = req.result;
				d.createObjectStore('projects', { keyPath: 'id' });
				d.createObjectStore('rows', { keyPath: 'id' }).createIndex('pid', 'pid');
			};
			req.onsuccess = () => ok(req.result);
			req.onerror = () => fail(req.error);
		});
	const wrap = (req) => new Promise((ok, fail) => { req.onsuccess = () => ok(req.result); req.onerror = () => fail(req.error); });
	const store = (name, mode = 'readonly') => db.transaction(name, mode).objectStore(name);
	const allProjects = () => wrap(store('projects').getAll());
	const getProject = (id) => wrap(store('projects').get(id));
	const getRow = (id) => wrap(store('rows').get(id));
	const putProject = (p) => wrap(store('projects', 'readwrite').put(p));
	async function rowsOf(pid) {
		const rows = await wrap(store('rows').index('pid').getAll(pid));
		return rows.sort((a, b) => a.created.localeCompare(b.created));
	}
	async function putRow(row) {
		await wrap(store('rows', 'readwrite').put(row));
		await touch(row.pid);
		askPersist();
	}
	async function deleteRow(row) {
		await wrap(store('rows', 'readwrite').delete(row.id));
		await touch(row.pid);
	}
	async function touch(pid) {
		const p = await getProject(pid);
		if (p) { p.updated = new Date().toISOString(); await putProject(p); }
	}
	async function deleteProject(p) {
		const tx = db.transaction(['projects', 'rows'], 'readwrite');
		const rows = tx.objectStore('rows');
		for (const key of await wrap(rows.index('pid').getAllKeys(p.id))) rows.delete(key);
		tx.objectStore('projects').delete(p.id);
		await new Promise((ok, fail) => { tx.oncomplete = ok; tx.onerror = () => fail(tx.error); });
	}
	let persisted = false;
	function askPersist() {
		if (!persisted && navigator.storage && navigator.storage.persist) navigator.storage.persist().then((v) => { persisted = v; });
	}

	// ---------- sample IDs ----------
	const letters = (k) => (k < 26 ? String.fromCharCode(65 + k) : letters(Math.floor(k / 26) - 1) + letters(k % 26));
	// Returns {id} or {missing: label} when a field the pattern needs is empty.
	function makeId(p, v, rows, selfId) {
		const pattern = p.pattern || '';
		if (!pattern.trim()) return { id: '' };
		const used = new Set(rows.filter((r) => r.id !== selfId).map((r) => r.v.sample));
		const fields = fieldsOf(p);
		const hasRep = pattern.includes('{rep}');
		const hasN = /\{n+\}/.test(pattern);
		const base = rows.filter((r) => r.id !== selfId).length + 1;
		const day = String(v.collected || v.date || today()).slice(0, 10).replace(/-/g, '');
		let missing = '';
		const fill = (k) =>
			pattern.replace(/\{([A-Za-z0-9_]+)\}/g, (all, key) => {
				if (key === 'rep') return letters(k);
				if (/^n+$/.test(key)) return pad(base + (hasRep ? 0 : k), key.length);
				if (key === 'date') return day;
				const f = fields.find((x) => x.k === key);
				if (!f) return all;
				const val = v[key] == null ? '' : String(v[key]).trim();
				if (!val) missing = missing || tr(f.l);
				return cleanName(val);
			});
		for (let k = 0; k < 2000; k++) {
			const id = fill(k);
			if (missing) return { missing };
			if (!used.has(id) || (!hasRep && !hasN)) return { id };
		}
		return { id: fill(0) };
	}

	// ---------- validation ----------
	// errors block saving; warnings (duplicate ID, odd characters) are shown and flagged in lists.
	function validate(p, v, rows, selfId) {
		const errors = {};
		const warnings = {};
		for (const f of fieldsOf(p)) {
			const val = v[f.k];
			const empty = val == null || val === '' || (f.t === 'gps' && !val.lat && val.lat !== 0);
			if (empty) {
				if (f.req) errors[f.k] = t('required');
				continue;
			}
			if (f.t === 'num') {
				const n = num(val);
				if (Number.isNaN(n)) errors[f.k] = t('notNumber');
				else if ((f.min != null && n < f.min) || (f.max != null && n > f.max)) errors[f.k] = t('range', f.min ?? '−∞', f.max ?? '∞');
			} else if (f.t === 'nums') {
				const parts = splitNums(val).map(num);
				if (!parts.length || parts.some(Number.isNaN)) errors[f.k] = t('numList');
				else if (parts.some((n) => n < f.min || n > f.max)) errors[f.k] = t('range', f.min, f.max);
			}
		}
		const id = (v.sample || '').trim();
		if (id) {
			if (/[^A-Za-z0-9._-]/.test(id)) warnings.sample = t('badChars');
			if (rows.some((r) => r.id !== selfId && (r.v.sample || '').trim() === id)) warnings.sample = t('dupId');
		}
		return { errors, warnings };
	}

	// ---------- CSV and JSON ----------
	function csvColumns(p) {
		const cols = [];
		for (const f of fieldsOf(p)) {
			if (f.t === 'photo') continue;
			if (f.t === 'gps') cols.push(['gps_lat', (v) => v.gps?.lat], ['gps_lon', (v) => v.gps?.lon], ['gps_accuracy_m', (v) => v.gps?.acc]);
			else if (f.t === 'num') cols.push([f.k, (v) => { const n = num(v[f.k] ?? ''); return Number.isNaN(n) ? v[f.k] : n; }]);
			else if (f.t === 'nums') cols.push([f.k, (v) => (v[f.k] ? splitNums(v[f.k]).map((s) => num(s)).join(';') : '')]);
			else cols.push([f.k, (v) => v[f.k]]);
		}
		return cols;
	}
	// Line breaks become " / ": isotable reads sheets line by line, so a quoted multi-line cell would
	// turn its second line into a sample.
	const csvCell = (x) => {
		const s = x == null ? '' : String(x).replace(/\s*\r?\n\s*/g, ' / ');
		return /[",\r\n]|^\s|\s$/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
	};
	function toCsv(p, rows) {
		const cols = csvColumns(p);
		const lines = [cols.map((c) => c[0]).join(',')];
		for (const r of rows) lines.push(cols.map((c) => csvCell(c[1](r.v))).join(','));
		return lines.join('\n') + '\n';
	}
	const blobToDataUrl = (b) => new Promise((ok) => { const fr = new FileReader(); fr.onload = () => ok(fr.result); fr.readAsDataURL(b); });
	function dataUrlToBlob(u) {
		const m = /^data:([^;,]+)?(;base64)?,(.*)$/s.exec(u || '');
		if (!m) return null;
		const bin = m[2] ? atob(m[3]) : decodeURIComponent(m[3]);
		const bytes = new Uint8Array(bin.length);
		for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
		return new Blob([bytes], { type: m[1] || 'application/octet-stream' });
	}
	async function backup(projects, withPhotos) {
		const out = { format: 'irit-fieldsheet', version: 1, exported: new Date().toISOString(), projects: [] };
		for (const p of projects) {
			const rows = [];
			for (const r of await rowsOf(p.id)) {
				const { photo, ...rest } = r;
				rows.push({ ...rest, photo: withPhotos && photo ? await blobToDataUrl(photo) : null });
			}
			out.projects.push({ ...p, rows });
		}
		return JSON.stringify(out, null, 1);
	}
	async function importBackup(text) {
		let data;
		try { data = JSON.parse(text); } catch { data = null; }
		if (!data || data.format !== 'irit-fieldsheet' || !Array.isArray(data.projects)) throw new Error(t('badBackup'));
		let added = 0, updated = 0, same = 0;
		for (const ip of data.projects) {
			if (!ip || typeof ip.id !== 'string' || !TEMPLATES[ip.template]) throw new Error(t('badBackup'));
			const { rows = [], ...proj } = ip;
			const have = await getProject(proj.id);
			if (!have || (proj.updated || '') > (have.updated || '')) await putProject(proj);
			for (const ir of rows) {
				if (!ir || typeof ir.id !== 'string' || !ir.v || typeof ir.v !== 'object') continue;
				const old = await getRow(ir.id);
				if (old && (ir.updated || '') <= (old.updated || '')) { same++; continue; }
				const row = { ...ir, pid: proj.id, photo: ir.photo ? dataUrlToBlob(ir.photo) : old?.photo || null };
				await wrap(store('rows', 'readwrite').put(row));
				old ? updated++ : added++;
			}
		}
		askPersist();
		return t('imported', data.projects.length, added, updated, same);
	}
	function download(name, text, type) {
		const url = URL.createObjectURL(new Blob([text], { type }));
		const a = h('a', { href: url, download: name });
		document.body.append(a);
		a.click();
		a.remove();
		setTimeout(() => URL.revokeObjectURL(url), 5000);
		toast(t('downloaded', name));
	}
	const stamp = () => today().replace(/-/g, '');

	// ---------- drafts (unsaved form contents survive reloads and the back button) ----------
	const draftKey = () => 'fieldsheet-draft:' + location.hash.replace(/\?.*$/, '');
	const saveDraft = (v) => localStorage.setItem(draftKey(), JSON.stringify(v));
	const takeDraft = () => { try { return JSON.parse(localStorage.getItem(draftKey())); } catch { return null; } };
	const dropDraft = () => localStorage.removeItem(draftKey());

	// ---------- views ----------
	const app = $('#app');
	function screen(title, back, ...kids) {
		$('#title').textContent = title;
		const b = $('#back');
		b.hidden = !back;
		b.onclick = () => { location.hash = back; };
		app.replaceChildren(...kids.flat(9).filter(Boolean));
		window.scrollTo(0, 0);
	}
	// One form field: its label holds only the name; hints and messages are linked with aria-describedby.
	let fieldSeq = 0;
	function fieldBox(f, el, ...notes) {
		const id = 'f' + ++fieldSeq;
		const named = el.matches('input, select, textarea');
		if (named) {
			el.id = id;
			el.setAttribute('aria-describedby', notes.filter(Boolean).map((n, i) => (n.id ||= `${id}-${i}`)).join(' '));
		}
		return h('div', { class: 'field', 'data-k': f.k },
			named ? h('label', { for: id, class: f.req ? 'req' : null }, tr(f.l)) : h('span', { class: 'lab' }, tr(f.l)), el, ...notes);
	}

	const photosBox = () => h('input', { type: 'checkbox', checked: localStorage.getItem('fieldsheet-photos') === '1',
		onchange: (e) => localStorage.setItem('fieldsheet-photos', e.target.checked ? '1' : '0') });

	async function viewHome() {
		const projects = (await allProjects()).sort((a, b) => (b.updated || '').localeCompare(a.updated || ''));
		const counts = await Promise.all(projects.map((p) => wrap(store('rows').index('pid').count(p.id))));
		const photos = photosBox();
		const file = h('input', { type: 'file', accept: 'application/json,.json', hidden: true, onchange: async (e) => {
			const f = e.target.files[0];
			if (!f) return;
			try { toast(await importBackup(await f.text())); render(); } catch (err) { toast(err.message); }
		} });
		const persistNote = h('p', { class: 'muted' });
		if (navigator.storage && navigator.storage.persisted) navigator.storage.persisted().then((v) => { persisted = v; persistNote.textContent = t(v ? 'persisted' : 'notPersisted'); });
		screen('fieldsheet', null,
			h('button', { class: 'primary big', type: 'button', onclick: () => { location.hash = '#/new'; } }, t('newProject')),
			h('h2', {}, t('projects')),
			projects.length
				? h('ul', { class: 'list' }, projects.map((p, i) => h('li', {}, h('button', { class: 'item', type: 'button', onclick: () => { location.hash = `#/p/${p.id}`; } },
					h('b', {}, p.name), h('span', { class: 'sub' }, `${t(TEMPLATES[p.template].name)} · ${t('nSamples', counts[i])}`)))))
				: h('p', { class: 'muted' }, t('noProjects')),
			h('h2', {}, t('backupAll')),
			h('div', { class: 'row-btns' },
				h('button', { type: 'button', disabled: !projects.length, onclick: async () => download(`fieldsheet-backup-${stamp()}.json`, await backup(projects, photos.checked), 'application/json') }, t('backupAll')),
				h('button', { type: 'button', onclick: () => file.click() }, t('importBackup'))),
			h('label', { class: 'check' }, photos, t('withPhotos')),
			file,
			h('p', { class: 'muted' }, t('localNote')),
			persistNote);
	}

	function viewNewProject() {
		const name = h('input', { required: true, autocomplete: 'off' });
		const pattern = h('input', { value: TEMPLATES.water.pattern, autocomplete: 'off', spellcheck: 'false' });
		const tpl = h('select', { onchange: () => { pattern.value = TEMPLATES[tpl.value].pattern; } },
			Object.entries(TEMPLATES).map(([k, x]) => h('option', { value: k }, t(x.name))));
		const err = h('p', { class: 'err', hidden: true }, t('required'));
		screen(t('newProject'), '#/',
			h('form', { onsubmit: async (e) => {
				e.preventDefault();
				if (!name.value.trim()) { err.hidden = false; name.focus(); return; }
				const now = new Date().toISOString();
				const p = { id: uid(), name: name.value.trim(), template: tpl.value, pattern: pattern.value.trim(), custom: [], gps: 'exact', created: now, updated: now };
				await putProject(p);
				location.hash = `#/p/${p.id}`;
			} },
				h('label', {}, h('span', { class: 'req' }, t('projName')), name, err),
				h('label', {}, t('template'), tpl),
				h('label', {}, t('pattern'), h('span', { class: 'hint' }, t('patternHint')), pattern),
				h('button', { class: 'primary', type: 'submit' }, t('create'))));
		name.focus();
	}

	function rowSummary(p, r) {
		const v = r.v;
		if (p.template === 'water') {
			const pt = POINTS.find((x) => x[0] === v.point);
			return [v.site, pt ? `${pt[0]} ${tr(pt.slice(1))}` : v.point, (v.collected || '').replace('T', ' ')].filter(Boolean).join(' · ');
		}
		return [v.source, v.species_guess, v.date].filter(Boolean).join(' · ');
	}

	async function viewProject(p) {
		const rows = await rowsOf(p.id);
		const counts = new Map();
		for (const r of rows) if (r.v.sample) counts.set(r.v.sample.trim(), (counts.get(r.v.sample.trim()) || 0) + 1);
		const dupIds = [...counts].filter(([, n]) => n > 1).map(([k]) => k);
		let nBad = 0;
		const items = rows.map((r) => {
			const { errors } = validate(p, r.v, rows, r.id);
			const errKeys = Object.keys(errors);
			const isDup = dupIds.includes((r.v.sample || '').trim());
			if (errKeys.length) nBad++;
			const names = errKeys.map((k) => tr(fieldsOf(p).find((f) => f.k === k)?.l) || k);
			return h('li', {}, h('button', { class: 'item', type: 'button', onclick: () => { location.hash = `#/p/${p.id}/row/${r.id}`; } },
				h('b', {}, r.v.sample || t('noId'), r.photo ? ' 📷' : ''),
				h('span', { class: 'sub' }, rowSummary(p, r)),
				isDup ? h('span', { class: 'flag' }, '⚠ ' + t('dupId')) : null,
				errKeys.length ? h('span', { class: 'flag' }, '⚠ ' + names.join(', ')) : null));
		});
		const exportCsv = () => {
			if (dupIds.length && !confirm(t('dupWarnExport', dupIds.join(', ')))) return null;
			return { name: `${slug(p.name)}-${stamp()}.csv`, text: toCsv(p, rows) };
		};
		const canShare = !!(navigator.canShare && navigator.canShare({ files: [new File([''], 'x.csv', { type: 'text/csv' })] }));
		const photos = photosBox();
		screen(p.name, '#/',
			h('button', { class: 'primary big', type: 'button', onclick: () => { location.hash = `#/p/${p.id}/add`; } }, t('addSample')),
			h('p', { class: 'count' }, [t('nSamples', rows.length), dupIds.length ? t('dups', rows.filter((r) => dupIds.includes((r.v.sample || '').trim())).length) : '', nBad ? t('problems', nBad) : ''].filter(Boolean).join(' · ')),
			rows.length ? h('ul', { class: 'list' }, items) : h('p', { class: 'muted' }, t('noRows')),
			h('h2', {}, t('exportCsv')),
			h('p', { class: 'muted' }, t('csvNote')),
			h('div', { class: 'row-btns' },
				h('button', { type: 'button', disabled: !rows.length, onclick: () => { const c = exportCsv(); if (c) download(c.name, c.text, 'text/csv;charset=utf-8'); } }, t('exportCsv')),
				canShare ? h('button', { type: 'button', disabled: !rows.length, onclick: async () => {
					const c = exportCsv();
					if (!c) return;
					try { await navigator.share({ files: [new File([c.text], c.name, { type: 'text/csv' })], title: c.name }); } catch (e) { if (e.name !== 'AbortError') toast(e.message); }
				} }, t('shareCsv')) : null,
				h('button', { type: 'button', onclick: async () => download(`${slug(p.name)}-backup-${stamp()}.json`, await backup([p], photos.checked), 'application/json') }, t('backupProject'))),
			h('label', { class: 'check' }, photos, t('withPhotos')),
			h('div', { class: 'row-btns' }, h('button', { type: 'button', onclick: () => { location.hash = `#/p/${p.id}/settings`; } }, t('settings'))));
	}

	function viewSettings(p, nRows) {
		const name = h('input', { value: p.name, required: true });
		const pattern = h('input', { value: p.pattern || '', spellcheck: 'false', autocomplete: 'off' });
		const custom = (p.custom || []).slice();
		const list = h('ul', { class: 'list' });
		const drawList = () => list.replaceChildren(...(custom.length ? custom.map((f, i) => h('li', { class: 'inline' },
			h('span', { class: 'item btn', style: 'flex:1' }, h('b', {}, tr(f.l)), h('span', { class: 'sub' }, `${f.k} · ${t({ text: 'tText', num: 'tNum', date: 'tDate', sel: 'tSel' }[f.t])}${f.req ? ' · ' + t('required') : ''}`)),
			h('button', { type: 'button', class: 'danger', onclick: () => { if (confirm(t('removeFieldQ', tr(f.l)))) { custom.splice(i, 1); drawList(); } } }, t('remove'))))
			: [h('li', { class: 'muted' }, t('noCustom'))]));
		drawList();
		const fl = h('input', { autocomplete: 'off' });
		const ft = h('select', {}, [['text', 'tText'], ['num', 'tNum'], ['date', 'tDate'], ['sel', 'tSel']].map(([v, k]) => h('option', { value: v }, t(k))));
		const fmin = h('input', { inputmode: 'decimal' });
		const fmax = h('input', { inputmode: 'decimal' });
		const fopts = h('input', { autocomplete: 'off' });
		const freq = h('input', { type: 'checkbox' });
		const ferr = h('p', { class: 'err', hidden: true });
		const numBox = h('div', { class: 'inline', hidden: true }, h('label', {}, t('min'), fmin), h('label', {}, t('max'), fmax));
		const optBox = h('label', { hidden: true }, t('fieldOptions'), fopts);
		ft.onchange = () => { numBox.hidden = ft.value !== 'num'; optBox.hidden = ft.value !== 'sel'; };
		const addField = () => {
			const lab = fl.value.trim();
			if (!lab) { ferr.textContent = t('required'); ferr.hidden = false; return; }
			const key = slug(lab);
			if (fieldsOf({ ...p, custom }).some((f) => f.k === key) || ['gps_lat', 'gps_lon', 'gps_accuracy_m'].includes(key)) { ferr.textContent = t('keyExists'); ferr.hidden = false; return; }
			const f = { k: key, t: ft.value, l: lab, req: freq.checked };
			if (ft.value === 'num') {
				if (!Number.isNaN(num(fmin.value))) f.min = num(fmin.value);
				if (!Number.isNaN(num(fmax.value))) f.max = num(fmax.value);
			}
			if (ft.value === 'sel') f.opts = fopts.value.split(';').map((s) => s.trim()).filter(Boolean).map((s) => [s, s, s]);
			custom.push(f);
			fl.value = fmin.value = fmax.value = fopts.value = '';
			freq.checked = false;
			ferr.hidden = true;
			drawList();
		};
		const gps = h('select', {}, [['exact', 'gpsExact'], ['coarse', 'gpsCoarse'], ['off', 'gpsOff']].map(([v, k]) => h('option', { value: v, selected: p.gps === v }, t(k))));
		screen(t('settings'), `#/p/${p.id}`,
			h('form', { onsubmit: async (e) => {
				e.preventDefault();
				if (!name.value.trim()) { name.focus(); return; }
				Object.assign(p, { name: name.value.trim(), pattern: pattern.value.trim(), custom, gps: gps.value, updated: new Date().toISOString() });
				await putProject(p);
				toast(t('settingsSaved'));
				location.hash = `#/p/${p.id}`;
			} },
				h('label', {}, h('span', { class: 'req' }, t('projName')), name),
				h('label', {}, t('pattern'), h('span', { class: 'hint' }, t('patternHint')), pattern),
				p.template === 'water' ? h('label', {}, t('gpsMode'), gps) : null,
				h('fieldset', {}, h('legend', {}, t('customFields')), list,
					h('div', { style: 'display:grid;gap:.6rem;margin-top:.8rem' },
						h('label', {}, t('fieldLabel'), fl), h('label', {}, t('fieldType'), ft), numBox, optBox,
						h('label', { class: 'check' }, freq, t('required')), ferr,
						h('button', { type: 'button', onclick: addField }, t('addField')))),
				h('p', { class: 'muted' }, `${t('csvCols')}: ${csvColumns({ ...p, custom }).map((c) => c[0]).join(', ')}`),
				h('div', { class: 'sticky-save' }, h('button', { class: 'primary', type: 'submit' }, t('save')))),
			h('div', { class: 'row-btns', style: 'margin-top:2rem' }, h('button', { type: 'button', class: 'danger', onclick: async () => {
				if (!confirm(t('confirmDeleteProject', p.name, nRows))) return;
				await deleteProject(p);
				toast(t('deleted', p.name));
				location.hash = '#/';
			} }, t('deleteProject'))));
	}

	// Photo: shrunk to at most 1600 px on the long side, JPEG, so a phone's storage lasts.
	async function shrinkPhoto(file) {
		try {
			const bmp = await createImageBitmap(file);
			const s = Math.min(1, 1600 / Math.max(bmp.width, bmp.height));
			const c = h('canvas', { width: Math.round(bmp.width * s), height: Math.round(bmp.height * s) });
			c.getContext('2d').drawImage(bmp, 0, 0, c.width, c.height);
			return await new Promise((ok) => c.toBlob((b) => ok(b || file), 'image/jpeg', 0.82));
		} catch {
			return file;
		}
	}

	// mode: 'edit' (row), 'add' (prefilled from the last row), 'dup' (copy of row)
	async function viewRow(p, mode, rowId) {
		const rows = await rowsOf(p.id);
		const fields = fieldsOf(p);
		let row = null;
		let note = '';
		let v = {};
		let photo = null;
		if (mode === 'edit') {
			row = rows.find((r) => r.id === rowId);
			if (!row) { location.hash = `#/p/${p.id}`; return; }
			v = structuredClone(row.v);
			photo = row.photo || null;
		} else if (mode === 'dup') {
			const src = rows.find((r) => r.id === rowId);
			if (src) { v = structuredClone(src.v); note = t('copyOf', src.v.sample || t('noId')); }
			v.sample = '';
		} else if (rows.length) {
			v = structuredClone(rows[rows.length - 1].v);
			v.sample = '';
			note = t('prefilled');
		}
		if (mode === 'add') for (const f of fields) if (f.auto) v[f.k] = f.t === 'dt' ? nowLocal() : today();
		if (mode !== 'edit') for (const f of fields) if (f.auto && !v[f.k]) v[f.k] = f.t === 'dt' ? nowLocal() : today();
		const draft = takeDraft();
		if (draft && typeof draft === 'object') { v = draft; note = t('draftRestored'); }
		let autoId = mode !== 'edit' && !v.sample;

		const inputs = {};
		const errEls = {};
		const summary = h('div', { class: 'summary', hidden: true, role: 'alert' });
		const sampleHint = h('span', { class: 'hint', 'aria-live': 'polite' });
		const read = () => {
			const out = { ...v };
			for (const f of fields) {
				const el = inputs[f.k];
				if (!el || f.t === 'gps' || f.t === 'photo') continue;
				out[f.k] = el.value.trim();
			}
			return out;
		};
		const refreshId = () => {
			const cur = read();
			const r = makeId(p, cur, rows, row?.id);
			if (autoId) inputs.sample.value = r.id || '';
			sampleHint.textContent = r.missing ? t('tokenMissing', r.missing) : p.pattern ? t('autoId', p.pattern) : '';
		};
		const showChecks = (blocking) => {
			const cur = read();
			const { errors, warnings } = validate(p, cur, rows, row?.id);
			for (const f of fields) {
				const msg = errors[f.k] || warnings[f.k] || '';
				const show = errors[f.k] ? (blocking || cur[f.k] !== '' && cur[f.k] != null) : !!warnings[f.k];
				errEls[f.k].textContent = show ? msg : '';
				errEls[f.k].className = errors[f.k] ? 'err' : 'note';
				errEls[f.k].parentElement.classList.toggle('invalid', show && !!errors[f.k]);
				inputs[f.k]?.setAttribute('aria-invalid', show && errors[f.k] ? 'true' : 'false');
			}
			return errors;
		};

		const gpsMode = p.gps || 'exact';
		const gpsOut = h('span', { class: 'hint' });
		const drawGps = () => { gpsOut.textContent = v.gps ? `${v.gps.lat}, ${v.gps.lon} (±${v.gps.acc} m)` : ''; };
		const photoBox = h('div', {});
		const drawPhoto = () => {
			const old = photoBox.querySelector('img');
			if (old) URL.revokeObjectURL(old.src);
			photoBox.replaceChildren(photo ? h('img', { class: 'thumb', alt: '', src: URL.createObjectURL(photo) }) : '',
				photo ? h('button', { type: 'button', class: 'danger', onclick: () => { photo = null; drawPhoto(); } }, t('photoRemove')) : '');
		};
		const controls = fields.map((f) => {
			let el;
			const val = v[f.k] ?? '';
			if (f.t === 'sel') {
				el = h('select', {}, h('option', { value: '' }, '—'),
					(f.opts || []).map((o) => h('option', { value: o[0], selected: val === o[0] }, o[0] === o[1] ? o[0] : `${o[0]}: ${tr(o.slice(1))}`)));
				if (val && !(f.opts || []).some((o) => o[0] === val)) el.append(h('option', { value: val, selected: true }, val));
			} else if (f.t === 'area') el = h('textarea', { value: val });
			else if (f.t === 'num' || f.t === 'nums') el = h('input', { value: val, inputmode: 'decimal', autocomplete: 'off' });
			else if (f.t === 'dt') el = h('input', { type: 'datetime-local', value: val });
			else if (f.t === 'date') el = h('input', { type: 'date', value: val });
			else if (f.t === 'gps') {
				const btn = h('button', { type: 'button', disabled: gpsMode === 'off' || !navigator.geolocation, onclick: () => {
					gpsOut.textContent = t('gpsWait');
					navigator.geolocation.getCurrentPosition((pos) => {
						const c = pos.coords;
						const coarse = gpsMode === 'coarse';
						const r = (x) => Number(x.toFixed(coarse ? 2 : 6));
						v.gps = { lat: r(c.latitude), lon: r(c.longitude), acc: Math.round(coarse ? Math.max(c.accuracy, 1100) : c.accuracy) };
						drawGps();
						saveDraft(read());
					}, (e) => { gpsOut.textContent = t('gpsFail', e.message); }, { enableHighAccuracy: gpsMode === 'exact', timeout: 30000, maximumAge: 60000 });
				} }, '📍 ' + t('gpsGet'));
				drawGps();
				el = h('div', {}, h('div', { class: 'inline' }, btn, h('button', { type: 'button', onclick: () => { delete v.gps; drawGps(); saveDraft(read()); } }, t('clear'))),
					h('span', { class: 'hint' }, `${t('gpsMode')}: ${t({ exact: 'gpsExact', coarse: 'gpsCoarse', off: 'gpsOff' }[gpsMode])}`), gpsOut);
			} else if (f.t === 'photo') {
				const fileIn = h('input', { type: 'file', accept: 'image/*', capture: 'environment', hidden: true, onchange: async (e) => {
					const file = e.target.files[0];
					if (file) { photo = await shrinkPhoto(file); drawPhoto(); }
				} });
				drawPhoto();
				el = h('div', {}, h('button', { type: 'button', onclick: () => fileIn.click() }, '📷 ' + t('photoAdd')), fileIn, h('span', { class: 'hint' }, t('photoNote')), photoBox);
			} else el = h('input', { value: val, autocomplete: 'off', spellcheck: f.k === 'sample' ? 'false' : null, autocapitalize: f.k === 'sample' || f.k === 'site' ? 'characters' : null });
			if (f.t !== 'gps' && f.t !== 'photo') inputs[f.k] = el;
			errEls[f.k] = h('span', { class: 'err' });
			return fieldBox(f, el, f.h ? h('span', { class: 'hint' }, tr(f.h)) : null, f.k === 'sample' ? sampleHint : null, errEls[f.k]);
		});

		const save = async (next) => {
			if (autoId && !inputs.sample.value) refreshId();
			const errors = showChecks(true);
			const keys = Object.keys(errors);
			if (keys.length) {
				summary.replaceChildren(h('b', {}, t('fixErrors')), h('ul', {}, keys.map((k) => h('li', {}, `${tr(fields.find((f) => f.k === k).l)}: ${errors[k]}`))));
				summary.hidden = false;
				summary.scrollIntoView({ block: 'center' });
				const first = inputs[keys[0]];
				if (first) first.focus({ preventScroll: true });
				return;
			}
			const now = new Date().toISOString();
			const out = read();
			for (const k of Object.keys(out)) if (out[k] === '' || out[k] == null) delete out[k];
			const rec = { id: row?.id || uid(), pid: p.id, v: out, photo, created: row?.created || now, updated: now };
			await putRow(rec);
			dropDraft();
			toast(t('saved', out.sample || ''));
			location.hash = next ? `#/p/${p.id}/add?${Date.now()}` : `#/p/${p.id}`;
		};

		const form = h('form', { novalidate: true, onsubmit: (e) => { e.preventDefault(); save(false); },
			oninput: (e) => {
				if (e.target === inputs.sample) autoId = !inputs.sample.value;
				else if (autoId) refreshId();
				showChecks(false);
				saveDraft(read());
			},
			onchange: () => { if (autoId) refreshId(); saveDraft(read()); } },
			note ? h('p', { class: 'note' }, note) : null,
			summary,
			controls,
			h('div', { class: 'sticky-save' },
				h('button', { class: 'primary', type: 'submit' }, t('save')),
				h('button', { type: 'button', onclick: () => save(true) }, t('saveNext'))));
		const extra = row ? h('div', { class: 'row-btns' },
			h('button', { type: 'button', onclick: () => { location.hash = `#/p/${p.id}/dup/${row.id}`; } }, t('duplicate')),
			h('button', { type: 'button', class: 'danger', onclick: async () => {
				if (!confirm(t('confirmDeleteRow', row.v.sample || t('noId')))) return;
				await deleteRow(row);
				dropDraft();
				toast(t('deleted', row.v.sample || ''));
				location.hash = `#/p/${p.id}`;
			} }, t('delete'))) : null;
		screen(row ? row.v.sample || t('noId') : t('addSample').replace('+ ', ''), `#/p/${p.id}`, form, extra);
		const back = $('#back');
		back.onclick = () => { dropDraft(); location.hash = `#/p/${p.id}`; };
		refreshId();
		showChecks(false);
	}

	// ---------- routing ----------
	async function render() {
		document.documentElement.lang = lang;
		const lb = $('#lang');
		lb.textContent = lang === 'id' ? 'EN' : 'ID';
		lb.setAttribute('aria-label', t('langName'));
		$('#back').setAttribute('aria-label', t('back'));
		const parts = location.hash.replace(/^#\/?/, '').replace(/\?.*$/, '').split('/').filter(Boolean);
		if (parts[0] === 'new') return viewNewProject();
		if (parts[0] === 'p' && parts[1]) {
			const p = await getProject(parts[1]);
			if (!p) { location.hash = '#/'; return; }
			if (parts[2] === 'settings') return viewSettings(p, (await rowsOf(p.id)).length);
			if (parts[2] === 'add') return viewRow(p, 'add');
			if (parts[2] === 'row' && parts[3]) return viewRow(p, 'edit', parts[3]);
			if (parts[2] === 'dup' && parts[3]) return viewRow(p, 'dup', parts[3]);
			return viewProject(p);
		}
		return viewHome();
	}

	$('#lang').addEventListener('click', () => {
		lang = lang === 'id' ? 'en' : 'id';
		localStorage.setItem('fieldsheet-lang', lang);
		render();
	});
	window.addEventListener('hashchange', render);

	openDb()
		.then((d) => { db = d; return render(); })
		.catch(() => { app.replaceChildren(h('p', { class: 'pad err' }, t('noIdb'))); });
	if ('serviceWorker' in navigator && location.protocol !== 'file:') navigator.serviceWorker.register('sw.js').catch(() => {});
})();
