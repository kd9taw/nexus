# DeepCW model (AI CW decoder, beta)

`model.onnx` + `model.onnx.json` come from https://github.com/e04/deepcw-engine
(**AGPL-3.0-only**, © e04) and are deliberately **not committed** to this repo.

The bundled copy is the upstream model **constant-folded at the app's fixed 15 s
decode window** (1001 spectrogram frames) with onnx-simplifier, because tract
cannot type-infer the graph's `Range` node over a symbolic time dimension:

```bash
pip install onnx onnxsim
python -c "
import onnx
from onnxsim import simplify
m = onnx.load('deepcw-engine/model.onnx')
s, ok = simplify(m, overwrite_input_shapes={'spectrogram': [1, 1, 1001, 65]})
assert ok
onnx.save(s, 'model.onnx')
"
cp deepcw-engine/model.onnx.json .
```

Semantics are unchanged (folding only bakes shapes/constants). Distributing the
model alongside Nexus is permitted: GPLv3 §13 ↔ AGPLv3 §13 allow the combination;
see NOTICE. If these files are absent the AI CW panel reports "model not
installed" and everything else works normally.
