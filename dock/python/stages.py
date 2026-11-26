"""Python side of `dock`: ligand prep, receptor prep, interaction profiling, RMSD.

Called by the Rust crate with `python -I stages.py <stage> ...`, one JSON object per run
written to stdout. Everything else goes to stderr so the caller can parse stdout blindly.

Two traps are handled here on purpose, because both cost an afternoon to find:

  * `openbabel` and `openbabel-wheel` both install a module called `openbabel`. With both
    present the shared library is clobbered and the interpreter dies with "double free or
    corruption" on import. `dock env` pins only openbabel-wheel; `check` below fails loudly
    if the other one reappears.

  * MDAnalysis guesses bonds by distance, and on a protonated protein that produces
    hydrogens with two bonds, which RDKit then refuses to sanitize. No fudge factor fixes
    it (0.45-0.70 all fail, in different places). We use RDKit's PDB reader instead.
    Note what that is and is not: RDKit also bonds by proximity ("connect the dots"), not
    from residue templates -- only bond *orders* for recognised standard residues come from
    a table. It works here because it respects covalent radii and valence limits, not
    because it knows what a residue looks like.
"""

import argparse, json, sys, warnings

warnings.filterwarnings("ignore")


def _out(obj):
    json.dump(obj, sys.stdout)
    sys.stdout.write("\n")


def check(_a):
    """Report versions, and refuse the openbabel double-install."""
    import importlib.metadata as md

    installed = {d.metadata["Name"].lower() for d in md.distributions() if d.metadata["Name"]}
    if "openbabel" in installed and "openbabel-wheel" in installed:
        raise SystemExit(
            "both 'openbabel' and 'openbabel-wheel' are installed; they provide the same "
            "module and crash the interpreter on import. Keep openbabel-wheel only."
        )
    vers = {}
    for name in ("rdkit", "meeko", "prolif", "pdb2pqr", "MDAnalysis", "numpy"):
        try:
            vers[name] = md.version(name)
        except md.PackageNotFoundError:
            vers[name] = None
    _out({"versions": vers})


def prep_ligand(a):
    from rdkit import Chem
    from rdkit.Chem import AllChem, rdMolDescriptors
    from meeko import MoleculePreparation, PDBQTWriterLegacy

    warnings_out = []
    if a.smiles:
        mol = Chem.MolFromSmiles(a.smiles)
        if mol is None:
            raise SystemExit(f"could not parse SMILES: {a.smiles}")
        warnings_out += _stereo_warnings(mol)
        mol = Chem.AddHs(mol)
        ps = AllChem.ETKDGv3()
        ps.randomSeed = a.seed
        if AllChem.EmbedMolecule(mol, ps) != 0:
            raise SystemExit("3D embedding failed; the molecule may be over-constrained")
        # -1 means MMFF has no parameters for this molecule and nothing was minimised;
        # 1 means it ran out of iterations. Saying "MMFF94 minimised" either way is a lie.
        rc = AllChem.MMFFOptimizeMolecule(mol, maxIters=2000)
        if rc == -1:
            source = f"SMILES, ETKDGv3 seed {a.seed}, NOT minimised (no MMFF94 parameters)"
            warnings_out.append(
                "MMFF94 has no parameters for this molecule; the conformer is the raw "
                "ETKDG embedding. Check the geometry before trusting the pose."
            )
        elif rc == 1:
            source = f"SMILES, ETKDGv3 seed {a.seed}, MMFF94 not converged in 2000 steps"
            warnings_out.append("MMFF94 minimisation did not converge in 2000 steps.")
        else:
            source = f"SMILES, ETKDGv3 seed {a.seed}, MMFF94 minimised"
    else:
        mol = _read_3d(a.input, a.template_smiles)
        warnings_out += _stereo_warnings(mol)
        mol = Chem.AddHs(mol, addCoords=True)
        source = f"3D coordinates from {a.input}"

    prep = MoleculePreparation()
    setups = prep.prepare(mol)
    pdbqt, ok, err = PDBQTWriterLegacy.write_string(setups[0])
    if not ok:
        raise SystemExit(f"PDBQT conversion failed: {err}")
    with open(a.out, "w") as fh:
        fh.write(pdbqt)
    _out({
        "pdbqt": a.out,
        "heavy_atoms": mol.GetNumHeavyAtoms(),
        "rotatable_bonds": rdMolDescriptors.CalcNumRotatableBonds(mol),
        "source": source,
        "warnings": warnings_out,
    })


def _stereo_warnings(mol):
    """Undefined stereocentres are resolved arbitrarily, so say so.

    ETKDG honours assigned stereochemistry and invents the rest. The seed makes the choice
    reproducible, not correct: docking (2R,3R)-epicatechin and (2R,3S)-catechin are different
    experiments, and a SMILES without @ markers silently picks one.
    """
    from rdkit.Chem import FindMolChiralCenters

    out = []
    unassigned = [c for c in FindMolChiralCenters(mol, includeUnassigned=True, useLegacyImplementation=False)
                  if c[1] == "?"]
    if unassigned:
        idx = ", ".join(str(i) for i, _ in unassigned)
        out.append(
            f"{len(unassigned)} stereocentre(s) are undefined (atom {idx}); one diastereomer "
            f"was chosen arbitrarily. Give the stereochemistry in the SMILES if it matters."
        )
    return out



def _read_3d(path, template_smiles=None):
    """Read a ligand that already has 3D coordinates.

    A PDB carries no bond orders: RDKit's parser bonds HETATM ligands by proximity and makes
    every bond single, so a benzene ring comes back as cyclohexane, AddHs fills the invented
    valences, and Meeko then types an aromatic ring as sp3 with rotatable bonds. Nothing
    errors. So a .pdb ligand needs a template SMILES to restore the bond orders, and without
    one we refuse rather than dock a molecule that is not the one the user meant.
    """
    from rdkit import Chem
    from rdkit.Chem import AllChem

    p = path.lower()
    if p.endswith(".sdf") or p.endswith(".mol"):
        mol = next(Chem.SDMolSupplier(path, removeHs=False), None)
    elif p.endswith(".mol2"):
        mol = Chem.MolFromMol2File(path, removeHs=False)
    elif p.endswith(".pdb"):
        if not template_smiles:
            raise SystemExit(
                f"{path} is a PDB, which carries no bond orders: every bond would be read as "
                f"single and the ligand docked as the wrong molecule. Pass --template-smiles "
                f"with the ligand's SMILES, or supply an SDF instead."
            )
        mol = Chem.MolFromPDBFile(path, removeHs=False)
        if mol is not None:
            try:
                mol = AllChem.AssignBondOrdersFromTemplate(Chem.MolFromSmiles(template_smiles), mol)
            except ValueError as e:
                raise SystemExit(
                    f"--template-smiles does not match the atoms in {path} ({e}). The template "
                    f"must be the same molecule, atom for atom; check for missing atoms in the "
                    f"structure or a different protonation state."
                )
    else:
        raise SystemExit(f"unsupported ligand format: {path} (use .sdf, .mol2 or .pdb)")
    if mol is None:
        raise SystemExit(f"could not read a molecule from {path}")
    if mol.GetNumConformers() == 0:
        raise SystemExit(f"{path} has no 3D coordinates; pass --smiles to generate them")
    return mol


def prep_receptor(a):
    """Protonate with pdb2pqr/PROPKA, then write PDBQT with Meeko."""
    import subprocess, os

    protonated = os.path.join(a.out_dir, "receptor_h.pdb")
    pqr = os.path.join(a.out_dir, "receptor.pqr")
    # `-m pdb2pqr`, not `pdb2pqr.main`: the latter has no __main__ guard, so it imports,
    # does nothing, and exits 0 without writing a file.
    cmd = [
        sys.executable, "-I", "-m", "pdb2pqr",
        "--ff=AMBER", f"--with-ph={a.ph}", "--titration-state-method=propka",
        # pdb2pqr keeps waters unless told otherwise, and a docking receptor is rigid and
        # dry by construction; leaving them in puts HOH contacts in the interaction report.
        "--drop-water",
        f"--pdb-output={protonated}", a.input, pqr,
    ]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0 or not os.path.exists(protonated):
        detail = (r.stderr or r.stdout or "").strip()[-1500:]
        raise SystemExit(
            f"pdb2pqr exited {r.returncode} without writing {protonated}.\n"
            f"{detail or '(it produced no output at all)'}"
        )

    base = os.path.join(a.out_dir, "receptor")
    from meeko import __file__ as _mk  # noqa: F401
    r2 = subprocess.run(
        [sys.executable, "-I", "-m", "meeko.cli.mk_prepare_receptor",
         "--read_pdb", protonated, "-o", base, "-p"],
        capture_output=True, text=True,
    )
    pdbqt = base + ".pdbqt"
    if not os.path.exists(pdbqt):
        # older Meeko ships the entry point as a script rather than a module
        r2 = subprocess.run(
            ["mk_prepare_receptor.py", "--read_pdb", protonated, "-o", base, "-p"],
            capture_output=True, text=True,
        )
    if not os.path.exists(pdbqt):
        raise SystemExit(f"receptor PDBQT not written:\n{r2.stdout[-800:]}\n{r2.stderr[-800:]}")

    nres = len({l[17:27] for l in open(protonated) if l.startswith("ATOM")})
    _out({"pdbqt": pdbqt, "protonated_pdb": protonated, "residues": nres, "ph": a.ph})


def _protein(path):
    """Protein as an RDKit mol. See the module docstring for why not MDAnalysis.

    Formal charges are not assigned here, so ProLIF's Anionic/Cationic terms -- which key off
    explicit formal charges -- may not fire. Salt bridges are therefore not something this
    report can be trusted to list. Documented as a limitation rather than papered over.
    """
    from rdkit import Chem, RDLogger

    RDLogger.DisableLog("rdApp.*")
    mol = Chem.MolFromPDBFile(path, removeHs=False, sanitize=False, proximityBonding=True)
    if mol is None:
        raise SystemExit(f"could not read receptor {path}")
    Chem.SanitizeMol(mol)
    return mol


def _pose(docked, index):
    from rdkit import Chem
    from meeko import PDBQTMolecule, RDKitMolCreate

    pm = PDBQTMolecule.from_file(docked, skip_typing=True)
    mol = RDKitMolCreate.from_pdbqt_mol(pm)[0]
    if index >= mol.GetNumConformers():
        raise SystemExit(f"pose {index + 1} not present; file has {mol.GetNumConformers()}")
    one = Chem.Mol(mol)
    one.RemoveAllConformers()
    one.AddConformer(mol.GetConformer(index), assignId=True)
    return one, mol


def profile(a):
    import prolif as plf

    lig_mol, _ = _pose(a.docked, a.pose - 1)
    lig = plf.Molecule.from_rdkit(lig_mol)
    prot = plf.Molecule.from_rdkit(_protein(a.receptor))
    fp = plf.Fingerprint(count=True)
    fp.run_from_iterable([lig], prot, progress=False)
    contacts = []
    for (_lres, pres), inters in fp.ifp[0].items():
        for kind, metas in inters.items():
            known = [m["distance"] for m in metas if m.get("distance") is not None]
            d = round(float(min(known)), 2) if known else None
            contacts.append({"residue": str(pres), "kind": kind, "distance": d})
    contacts.sort(key=lambda c: (c["residue"], c["kind"]))
    _out({"contacts": contacts})


def rmsd(a):
    """Symmetry-aware RMSD of every pose against a reference ligand."""
    from rdkit import Chem
    from rdkit.Chem import AllChem, rdMolAlign

    ref = Chem.MolFromPDBFile(a.reference, removeHs=True, sanitize=False)
    if ref is None:
        raise SystemExit(f"could not read reference {a.reference}")
    if a.smiles:
        try:
            ref = AllChem.AssignBondOrdersFromTemplate(Chem.MolFromSmiles(a.smiles), ref)
        except ValueError as e:
            raise SystemExit(
                f"--smiles does not match the deposited ligand atom for atom ({e}). Check for "
                f"unresolved atoms in the structure, or a different protonation state."
            )
    ref = Chem.RemoveHs(ref)

    _, mol = _pose(a.docked, 0)
    vals = []
    for i in range(mol.GetNumConformers()):
        one = Chem.Mol(mol)
        one.RemoveAllConformers()
        one.AddConformer(mol.GetConformer(i), assignId=True)
        vals.append(round(float(rdMolAlign.CalcRMS(Chem.RemoveHs(one), ref)), 3))
    _out({"rmsd": vals})


def box_from(a):
    """Docking box centred on a reference ligand, padded."""
    from rdkit import Chem

    mol = Chem.MolFromPDBFile(a.reference, removeHs=True, sanitize=False)
    if mol is None:
        raise SystemExit(f"could not read {a.reference}")
    c = mol.GetConformer()
    pts = [list(c.GetAtomPosition(i)) for i in range(mol.GetNumAtoms())]
    lo = [min(p[k] for p in pts) for k in range(3)]
    hi = [max(p[k] for p in pts) for k in range(3)]
    _out({
        "center": [round((lo[k] + hi[k]) / 2, 3) for k in range(3)],
        "size": [round(max(hi[k] - lo[k] + 2 * a.padding, 16.0), 1) for k in range(3)],
    })


def split(a):
    """Pull a named ligand out of a structure, and keep the protein separately."""
    import gemmi

    st = gemmi.read_structure(a.input)
    st.setup_entities()
    st.remove_alternative_conformations()
    st.remove_hydrogens()
    # Only the first model, and only the chain we are going to keep. Collecting every copy
    # across every chain would put 2+ ligands in the reference file, which makes box_from
    # span two sites and gives rmsd a reference with twice the atoms.
    found = []
    for model in st:
        for chain in model:
            for res in chain:
                if res.name == a.ligand:
                    found.append(f"{chain.name}/{res.seqid.num}")
        break
    if not found:
        raise SystemExit(f"ligand {a.ligand} not found in {a.input}")
    keep = a.chain or found[0].split("/")[0]
    kept = [f for f in found if f.split("/")[0] == keep]
    if not kept:
        raise SystemExit(f"ligand {a.ligand} is not in chain {keep}; it is in {found}")
    lig = gemmi.Structure()
    lig.add_model(gemmi.Model("1"))
    ch = gemmi.Chain("L")
    for model in st:
        for chain in model:
            if chain.name != keep:
                continue
            for res in chain:
                if res.name == a.ligand and f"{chain.name}/{res.seqid.num}" == kept[0]:
                    ch.add_residue(res)
        break
    lig[0].add_chain(ch)
    lig.setup_entities()
    lig.write_pdb(a.out_ligand)
    # Removes every non-polymer residue: waters, but also metal ions and cofactors.
    st.remove_ligands_and_waters()
    for model in st:
        for chain in list(model):
            if chain.name != keep:
                model.remove_chain(chain.name)
    st.setup_entities()
    st.write_pdb(a.out_receptor)
    _out({"ligand_copies": found, "receptor_chain": keep, "ligand_used": kept[0]})


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="stage", required=True)

    sub.add_parser("check").set_defaults(fn=check)

    p = sub.add_parser("prep-ligand"); p.set_defaults(fn=prep_ligand)
    p.add_argument("--smiles"); p.add_argument("--input")
    p.add_argument("--out", required=True); p.add_argument("--seed", type=int, default=0xC0FFEE)
    p.add_argument("--template-smiles", dest="template_smiles")

    p = sub.add_parser("prep-receptor"); p.set_defaults(fn=prep_receptor)
    p.add_argument("--input", required=True); p.add_argument("--out-dir", required=True)
    p.add_argument("--ph", type=float, default=7.4)

    p = sub.add_parser("profile"); p.set_defaults(fn=profile)
    p.add_argument("--receptor", required=True); p.add_argument("--docked", required=True)
    p.add_argument("--pose", type=int, default=1)

    p = sub.add_parser("rmsd"); p.set_defaults(fn=rmsd)
    p.add_argument("--docked", required=True); p.add_argument("--reference", required=True)
    p.add_argument("--smiles")

    p = sub.add_parser("box"); p.set_defaults(fn=box_from)
    p.add_argument("--reference", required=True); p.add_argument("--padding", type=float, default=5.0)

    p = sub.add_parser("split"); p.set_defaults(fn=split)
    p.add_argument("--input", required=True); p.add_argument("--ligand", required=True)
    p.add_argument("--out-receptor", required=True); p.add_argument("--out-ligand", required=True)
    p.add_argument("--chain")

    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
