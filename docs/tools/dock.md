# dock: molecular docking with a validation step

`dock` prepares a receptor and a ligand, runs AutoDock Vina, works out which residues the
best pose touches, and writes a single HTML report that depends on nothing outside itself.
It also redocks a ligand whose crystal pose is already known, so you can say how far the
method lands from the answer before you trust it on a molecule nobody has solved.

## Why

In silico screening is routine in pharmacy and biology theses: pick a natural product, dock
it against a disease target, report the binding affinity and the contacting residues. The
usual workflow is a sequence of GUI steps — a structure viewer, a preparation tool, the
docking program, a diagram tool — with nothing recording what was clicked. Two people
following the same published protocol get different numbers, and neither can say why.

Three failure modes show up again and again in that output:

* **The score is reported as a binding energy.** The Vina score is an empirical function
  fitted to a training set. It ranks poses of one molecule well. It ranks *different*
  molecules against each other much less well, and it is not a measured affinity. A
  difference of 0.5 kcal/mol between two compounds is not evidence that one binds tighter.
* **Only the best pose is shown.** Vina returns several, and in a typical run a good few sit
  well away from the top one — in a different part of the same pocket, or the molecule
  flipped end to end. A report that prints one number hides how little the search preferred
  its winner.
* **Nothing is validated.** If the protein has a known inhibitor with a solved structure,
  redocking it is a free control: a method that cannot reproduce a pose it has already been
  shown has no business ranking novel compounds.

`dock validate` exists for the third, and the report shows the pose spread for the second.
The first is a wording problem, so the report says "affinity" and explains what it is not.

## What it can and cannot show

**Can:** where a ligand plausibly sits in a defined pocket; which residues it contacts and
through what kind of interaction; how reproducible that pose is across the search; how far
the method lands from a crystal pose when one exists.

**Cannot:** whether the compound actually binds, how tightly, whether it is selective,
whether it is soluble, whether it reaches the target in a cell, or whether the protein even
adopts this conformation in solution. Docking searches a rigid receptor with no water. It
answers "does this shape fit here", and that is all.

**What this implementation in particular does not do**, each of which can change a result
without any error appearing:

* **The ligand is docked exactly as written.** `--receptor-ph` protonates the receptor and
  nothing else. A carboxylic acid given as `C(=O)O` docks neutral, an amine as `NH2` docks
  neutral. Vina decides hydrogen-bond donors and acceptors from the hydrogens present, so
  for anything that ionises near physiological pH you must write the charged form yourself.
  This is the most common way to get a confidently wrong pose out of this tool.
* **Undefined stereocentres are resolved arbitrarily.** The seed makes the choice repeatable,
  not correct. `dock` warns when it finds any; do not ignore the warning.
* **Metal ions and cofactors are removed with the waters.** `dock validate` strips every
  non-polymer residue, so zinc, magnesium, heme, NADPH and FAD are gone. For a
  metalloenzyme or a cofactor-dependent pocket the result is a hole where the chemistry was,
  and the score still looks ordinary. Those targets need manual preparation this tool does
  not provide.
* **Only one protein chain is kept.** A site that forms at a dimer interface is built
  half-missing.
* **Salt bridges are not reported.** The receptor is read without formal charges, so the
  fingerprint's ionic terms do not fire. Absence of an ionic contact in the report is not
  evidence of its absence in the pose.
* **One conformer is embedded per ligand.** Vina searches torsions itself, but the ring
  pucker it starts from is fixed for the whole run.

Reviewers increasingly want a molecular dynamics run and a binding free energy calculation
on top. Those are a different scale of computation — a 100 ns trajectory is tens of millions
of sequential force evaluations — and `dock` does not do them.

## Usage

```sh
# once: build the Python environment and fetch AutoDock Vina
dock env

# redock a known ligand and see how close the method gets
dock validate --structure 4asd.cif --ligand-code BAX \
  --smiles 'CNC(=O)c1cc(Oc2ccc(NC(=O)Nc3ccc(Cl)c(C(F)(F)F)c3)cc2)ccn1' \
  --out runs/sorafenib-control

# dock a new compound into the same pocket
# note the stereocentres: without @ markers this is "catechin or epicatechin, pick one"
dock run --receptor 4asd.cif \
  --smiles 'O[C@@H]1Cc2c(O)cc(O)cc2O[C@@H]1c1ccc(O)c(O)c1' \
  --name epicatechin --reference runs/sorafenib-control/crystal_ligand.pdb \
  --out runs/epicatechin
```

Run the control first. If the redock lands more than about 2 Å from the crystal pose,
nothing downstream of it is worth reporting.

### `dock env`

| Option | Effect |
| --- | --- |
| `--check` | Report what is installed without building anything |
| `--home <dir>` | Where the environment lives. Defaults to `$XDG_CACHE_HOME/dock` |
| `--vina <path>` | Use an existing AutoDock Vina binary instead of the managed one |

The environment is a `uv` virtualenv with pinned versions, plus the Vina binary verified
against a recorded SHA-256. Versions are pinned rather than floated so a result does not
change when a dependency releases.

### `dock run`

| Option | Effect |
| --- | --- |
| `--receptor <file>` | Receptor structure, PDB or mmCIF. Waters and other ligands are removed |
| `--smiles <s>` | Ligand as SMILES; 3D coordinates are generated |
| `--ligand-file <f>` | Ligand from a file that already has 3D coordinates (`.sdf`, `.mol2`, `.pdb`) |
| `--name <s>` | Label for the ligand in the report |
| `--center x,y,z` | Box centre in angstroms |
| `--size x,y,z` | Box size in angstroms. Default `24,24,24` |
| `--reference <f>` | Put the box around this bound ligand instead of giving a centre |
| `--padding <a>` | Padding around `--reference`. Default 5 Å |
| `--exhaustiveness <n>` | How hard Vina searches. Default 16 |
| `--num-modes <n>` | How many poses to keep. Default 9 |
| `--seed <n>` | Random seed. Fixed at 42 by default. Reproduces a run on the same machine with the same `--cpus`; Vina splits the seeded RNG across threads, so a different core count gives different poses |
| `--cpus <n>` | Threads. Defaults to every core |
| `--receptor-ph <n>` | pH used to protonate the **receptor**. Default 7.4. Does not touch the ligand |
| `--template-smiles <s>` | SMILES for `--ligand-file`, to restore bond orders. Required for a `.pdb` ligand |
| `--out <dir>` | Where to write results |
| `--overwrite` | Replace an existing output directory |

The box has to be chosen, not guessed. Pass `--reference` with a ligand that is already
bound in the pocket you care about, or `--center` from a pocket detection tool. A box that
covers the whole protein is a blind docking run, and its scores are not comparable to a
focused one.

### `dock validate`

Takes the same docking options, plus:

| Option | Effect |
| --- | --- |
| `--structure <f>` | A structure with the ligand still bound |
| `--ligand-code <c>` | Three-letter code of that ligand, e.g. `BAX` |
| `--smiles <s>` | SMILES for it, used to restore bond orders a PDB does not carry |
| `--chain <c>` | Which chain to keep. Defaults to the one holding the ligand |

RMSD is computed with symmetry taken into account, which matters: a phenyl ring flipped by
180° is the same structure, and comparing atoms by index would report a large error for an
identical pose.

## Outputs

| File | Contents |
| --- | --- |
| `report.html` | The figure and the pose table. No JavaScript, no network, no external files |
| `results.json` | Poses, contacts, RMSDs, ligand and receptor summary |
| `manifest.json` | Tool versions, every command in argv form, parameters, output hashes |
| `docked.pdbqt` | All poses, for a structure viewer |
| `receptor_h.pdb` | The protonated receptor the contacts were computed against |

The report deliberately embeds its figure as SVG rather than loading a graph library at view
time. ProLIF's own diagram fetches `vis-network` from a CDN, which renders an empty box with
no error when offline — not a property you want in a file attached to a thesis.

## How it is put together

Four stages behind traits: ligand preparation, receptor preparation, the docking engine, and
interaction profiling. The engine is AutoDock Vina. The other three are currently backed by
Python — RDKit, Meeko, ProLIF and pdb2pqr — because that stack is what the pipeline was
validated against.

Those stages are traits rather than functions because most of them have pure-Rust
replacements available now: `chematic` covers SMILES parsing, 3D embedding and Gasteiger
charges, and `pdbtbx` reads PDB and mmCIF. Moving a stage over removes a large dependency
and is considerably faster. It is also a good way to introduce a subtle chemistry error, so
a replacement is only allowed to become the default once it has been diffed against the
Python stage on the same inputs. That diffing harness does not exist yet and has to be built
before the first stage moves.

## Validation

The pipeline reproduces the crystallographic pose of sorafenib in VEGFR-2 (PDB 4ASD) at
**0.78 Å** RMSD in about 30 seconds on six cores. The pose that matches is the top-ranked
one, which is the honest benchmark — most sub-Ångström figures in the literature are
best-of-N across the returned poses.

The contacts recovered are sorafenib's published binding mode: two hydrogen bonds to the
Cys919 **backbone** at the hinge, the urea to Glu885, the urea carbonyl to the Asp1046
backbone NH, and the chloro-trifluoromethylphenyl group packed into the allosteric back
pocket (Leu889, Ile892, Leu1019, Ile1044) that the DFG-out conformation opens. That last
part is what makes sorafenib a type-II inhibitor — contacting Asp1046 and Phe1047 alone does
not, since plenty of type-I inhibitors touch the DFG aspartate too.

Six of the nine returned poses sit more than 5 Å from the best one — in a different part of
the same pocket, or flipped end to end — while still scoring between −8.8 and −9.9 kcal/mol.
That is normal, and it is the reason the report prints every pose rather than the winner.

Two caveats on what this control does and does not establish. The box is placed around the
crystal ligand itself, so the search is told where to look; that is standard practice for a
redocking control but it is not a blind test. And `dock validate` splits the structure with
gemmi before protonating, which `dock run` does not do, so the two paths do not prepare the
receptor identically.
