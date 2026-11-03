#version 450
// Diagnostic only: observe the original translated vertex shader's varying
// before texture sampling. Never used for guest rendering or pass substitution.
layout(location = 0) noperspective in vec4 varyingValue;
layout(location = 0) out vec4 result;
void main() {
    result = vec4(varyingValue.xy, 0.0, 1.0);
}
