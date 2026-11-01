#version 450
layout(push_constant) uniform Clear { vec4 color; } clear;
layout(location=0) out vec4 out_color;
void main() { out_color = clear.color; }
