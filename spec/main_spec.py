'''
Entry point spec for the torpedo fork.

libspec auto-discovery compiles spec/main_spec.py first; this Spec pulls in the
requirement/constraint classes defined in spec/design_spec.py.
'''

from libspec import Spec
from . import design_spec


class ForkSpec(Spec):
    def modules(self):
        return [design_spec]
