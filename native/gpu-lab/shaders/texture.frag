#version 450
layout(location=0) in vec2 uv;
layout(set=0,binding=0) uniform sampler2D convertedTexture;
layout(location=0) out vec4 color;
void main() {
    // Mirror X and sample beyond both image edges to exercise clamp-to-edge.
    color=texture(convertedTexture,vec2(1.25-1.5*uv.x,1.5*uv.y-0.25));
}
