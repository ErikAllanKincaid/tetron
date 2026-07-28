'''
Entry point spec for the tetron fork.

libspec auto-discovery compiles spec/main_spec.py first; this Spec pulls in the
requirement/constraint classes defined in spec/design_spec.py.
'''

from libspec import Spec
from . import addressing, branding, cli, constraints, core, membership, security


class ForkSpec(Spec):

  def modules(self):
    return [
        core,
        branding,
        addressing,
        membership,
        cli,
        security,
        constraints,
    ]

