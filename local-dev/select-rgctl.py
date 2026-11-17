#!/usr/bin/env python3
"""Select one local rgctl checkout for Cargo, provenance, and Podman builds."""
import argparse
from pathlib import Path
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('checkout',type=Path)
args=parser.parse_args()
source=args.checkout.resolve(strict=True)
if not (source/'crates/rgctl-graph/Cargo.toml').is_file():parser.error('Expected an rgctl source checkout')
link=Path(__file__).resolve().parents[1]/'rgctl-source'
if link.is_symlink():link.unlink()
elif link.exists():parser.error('Refusing to replace a non-symlink rgctl-source path')
link.symlink_to(source,target_is_directory=True)
print(f'Cargo and Podman now use {source}')
