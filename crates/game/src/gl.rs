//! WebGL2 renderer: two instanced programs (lit boxes, textured glyph quads)
//! and one glyph atlas. Four draw calls per frame plus a clear.

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    CanvasRenderingContext2d, HtmlCanvasElement, WebGl2RenderingContext as GL, WebGlBuffer,
    WebGlProgram, WebGlTexture, WebGlUniformLocation, WebGlVertexArrayObject,
};

use crate::math::Mat4;
use crate::scene::{
    ATLAS_COLS, ATLAS_ROWS, BG, BOX_FLOATS, Batches, CELL_GLOW, CELL_RING, CELL_SOLID, Font,
    GLYPH_FLOATS,
};

const BOX_VS: &str = r#"#version 300 es
layout(location=0) in vec3 a_pos;
layout(location=1) in vec3 a_nrm;
layout(location=2) in vec3 i_center;
layout(location=3) in vec3 i_size;
layout(location=4) in vec4 i_color;
layout(location=5) in float i_glow;
layout(location=6) in float i_style;
uniform mat4 u_vp;
out vec3 v_nrm;
out vec3 v_local;
out vec3 v_size;
out vec4 v_color;
out float v_glow;
out float v_style;
out float v_depth;
void main() {
    vec3 p = i_center + a_pos * i_size;
    gl_Position = u_vp * vec4(p, 1.0);
    v_nrm = a_nrm;
    v_local = a_pos;
    v_size = i_size;
    v_color = i_color;
    v_glow = i_glow;
    v_style = i_style;
    v_depth = -p.z;
}"#;

// Styles: 0 matte, 1 gem (bevelled, glossy, rim-lit), 2 ribbon (sustain:
// bright core with a shimmer running along it).
const BOX_FS: &str = r#"#version 300 es
precision mediump float;
in vec3 v_nrm;
in vec3 v_local;
in vec3 v_size;
in vec4 v_color;
in float v_glow;
in float v_style;
in float v_depth;
uniform vec3 u_fog_color;
uniform vec2 u_fog;
uniform float u_time;
out vec4 o;
void main() {
    vec3 l = normalize(vec3(0.25, 0.85, 0.55));
    float d = max(dot(v_nrm, l), 0.0);
    vec3 base = v_color.rgb;
    vec3 c = base * (0.42 + 0.58 * d) + base * v_glow;
    float a = v_color.a;
    if (v_style > 0.5 && v_style < 1.5) {
        // Gem: distance to the face edge in world units, for a bevel line,
        // a glossy upper half on the front and a bright rim.
        vec3 q = abs(v_local) * v_size;
        vec3 h = v_size * 0.5;
        vec3 e = h - q;
        float edge;
        if (abs(v_nrm.z) > 0.5) edge = min(e.x, e.y);
        else if (abs(v_nrm.y) > 0.5) edge = min(e.x, e.z);
        else edge = min(e.y, e.z);
        float bevel = 1.0 - smoothstep(0.0, 0.045, edge);
        float gloss = (v_nrm.z > 0.5) ? smoothstep(0.0, 0.5, v_local.y + 0.15) * 0.35 : 0.0;
        float top = (v_nrm.y > 0.5) ? 0.25 : 0.0;
        c = base * (0.55 + 0.45 * d + v_glow) + vec3(gloss + top);
        c = mix(c, base * 0.35 + vec3(0.9), bevel * 0.55);
    } else if (v_style > 1.5) {
        // Ribbon: brighter down the middle, shimmering along its length.
        float across = 1.0 - abs(v_local.x) * 2.0;
        float along = v_local.z * v_size.z;
        float shimmer = 0.5 + 0.5 * sin(along * 5.0 + u_time * 9.0);
        c = base * (0.6 + 0.5 * v_glow) * (0.75 + 0.5 * across) + vec3(0.25 * shimmer * across);
        a *= 0.55 + 0.45 * across;
    }
    // Far away, things darken a little and fade out into the backdrop.
    float f = smoothstep(u_fog.x, u_fog.y, v_depth);
    o = vec4(mix(c, u_fog_color, f * 0.5), a * (1.0 - f));
}"#;

const GLYPH_VS: &str = r#"#version 300 es
layout(location=0) in vec2 a_corner;
layout(location=1) in vec3 i_center;
layout(location=2) in vec2 i_size;
layout(location=3) in vec4 i_uv;
layout(location=4) in vec4 i_color;
layout(location=5) in float i_mode;
uniform mat4 u_vp;
uniform vec2 u_screen;
out vec2 v_uv;
out vec4 v_color;
out float v_depth;
void main() {
    vec2 c = a_corner - 0.5;
    if (i_mode > 0.5 && i_mode < 1.5) {
        vec2 px = i_center.xy + c * i_size;
        gl_Position = vec4(px.x / u_screen.x * 2.0 - 1.0, 1.0 - px.y / u_screen.y * 2.0, 0.0, 1.0);
        v_uv = vec2(mix(i_uv.x, i_uv.z, a_corner.x), mix(i_uv.y, i_uv.w, a_corner.y));
        v_depth = 0.0;
    } else {
        vec3 p = i_center + vec3(c * i_size, 0.0);
        gl_Position = u_vp * vec4(p, 1.0);
        v_uv = vec2(mix(i_uv.x, i_uv.z, a_corner.x), mix(i_uv.w, i_uv.y, a_corner.y));
        // Mode 2 (horizon haze) is never faded by distance.
        v_depth = i_mode > 1.5 ? 0.0 : -p.z;
    }
    v_color = i_color;
}"#;

const GLYPH_FS: &str = r#"#version 300 es
precision mediump float;
in vec2 v_uv;
in vec4 v_color;
in float v_depth;
uniform sampler2D u_tex;
uniform vec2 u_fog;
out vec4 o;
void main() {
    float a = texture(u_tex, v_uv).a;
    float f = smoothstep(u_fog.x, u_fog.y, v_depth);
    o = vec4(v_color.rgb, v_color.a * a * (1.0 - f));
}"#;

/// Backdrop: one full-screen triangle, cover-fitted, darkened, vignetted.
const BG_VS: &str = r#"#version 300 es
out vec2 v_uv;
void main() {
    vec2 p = vec2(gl_VertexID == 1 ? 3.0 : -1.0, gl_VertexID == 2 ? 3.0 : -1.0);
    v_uv = vec2(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    gl_Position = vec4(p, 0.0, 1.0);
}"#;

const BG_FS: &str = r#"#version 300 es
precision mediump float;
in vec2 v_uv;
uniform sampler2D u_tex;
uniform vec2 u_scale;
uniform vec3 u_base;
out vec4 o;
void main() {
    vec3 c = texture(u_tex, (v_uv - 0.5) * u_scale + 0.5).rgb;
    vec2 q = v_uv - vec2(0.5, 0.42);
    float vig = 1.0 - smoothstep(0.25, 0.85, length(q));
    o = vec4(mix(u_base, c * 0.42, 0.35 + 0.65 * vig), 1.0);
}"#;

/// Unit cube: 36 vertices of position(3) + normal(3).
fn cube() -> Vec<f32> {
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
    ];
    let mut v = Vec::with_capacity(36 * 6);
    for (n, u, w) in faces {
        let corner = |a: f32, b: f32| -> [f32; 3] {
            [
                n[0] * 0.5 + u[0] * a + w[0] * b,
                n[1] * 0.5 + u[1] * a + w[1] * b,
                n[2] * 0.5 + u[2] * a + w[2] * b,
            ]
        };
        let q = [
            corner(-0.5, -0.5),
            corner(0.5, -0.5),
            corner(0.5, 0.5),
            corner(-0.5, 0.5),
        ];
        for i in [0, 1, 2, 0, 2, 3] {
            v.extend_from_slice(&q[i]);
            v.extend_from_slice(&n);
        }
    }
    v
}

fn compile(gl: &GL, vs: &str, fs: &str) -> Result<WebGlProgram, JsValue> {
    let shader = |kind, src: &str| -> Result<_, JsValue> {
        let s = gl.create_shader(kind).ok_or("no shader")?;
        gl.shader_source(&s, src);
        gl.compile_shader(&s);
        if !gl
            .get_shader_parameter(&s, GL::COMPILE_STATUS)
            .as_bool()
            .unwrap_or(false)
        {
            return Err(gl.get_shader_info_log(&s).unwrap_or_default().into());
        }
        Ok(s)
    };
    let p = gl.create_program().ok_or("no program")?;
    gl.attach_shader(&p, &shader(GL::VERTEX_SHADER, vs)?);
    gl.attach_shader(&p, &shader(GL::FRAGMENT_SHADER, fs)?);
    gl.link_program(&p);
    if !gl
        .get_program_parameter(&p, GL::LINK_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        return Err(gl.get_program_info_log(&p).unwrap_or_default().into());
    }
    Ok(p)
}

struct Layer {
    vao: WebGlVertexArrayObject,
    buf: WebGlBuffer,
    /// Bytes allocated on the GPU for `buf`.
    cap: usize,
}

fn upload(gl: &GL, layer: &mut Layer, data: &[f32]) {
    gl.bind_buffer(GL::ARRAY_BUFFER, Some(&layer.buf));
    let bytes = data.len() * 4;
    if bytes > layer.cap {
        // Grow to the next power of two so this happens a handful of times only.
        layer.cap = bytes.next_power_of_two().max(4096);
        gl.buffer_data_with_i32(GL::ARRAY_BUFFER, layer.cap as i32, GL::DYNAMIC_DRAW);
    }
    if !data.is_empty() {
        // SAFETY: the view is consumed by the call below before any allocation.
        let view = unsafe { js_sys::Float32Array::view(data) };
        gl.buffer_sub_data_with_i32_and_array_buffer_view(GL::ARRAY_BUFFER, 0, &view);
    }
}

pub struct Renderer {
    gl: GL,
    canvas: HtmlCanvasElement,
    box_prog: WebGlProgram,
    glyph_prog: WebGlProgram,
    u_box_vp: Option<WebGlUniformLocation>,
    u_box_fog: Option<WebGlUniformLocation>,
    u_box_fog_color: Option<WebGlUniformLocation>,
    u_box_time: Option<WebGlUniformLocation>,
    u_gl_vp: Option<WebGlUniformLocation>,
    u_gl_screen: Option<WebGlUniformLocation>,
    u_gl_fog: Option<WebGlUniformLocation>,
    u_gl_tex: Option<WebGlUniformLocation>,
    opaque: Layer,
    clear: Layer,
    text: Layer,
    glow: Layer,
    hud: Layer,
    _atlas: WebGlTexture,
    pub font: Font,
    bg_prog: WebGlProgram,
    bg_vao: WebGlVertexArrayObject,
    u_bg_tex: Option<WebGlUniformLocation>,
    u_bg_scale: Option<WebGlUniformLocation>,
    u_bg_base: Option<WebGlUniformLocation>,
    /// The backdrop texture and its aspect ratio, once one is set.
    backdrop: Option<(WebGlTexture, f32)>,
}

impl Renderer {
    pub fn new(canvas: HtmlCanvasElement) -> Result<Renderer, JsValue> {
        let opts = js_sys::Object::new();
        for (k, v) in [
            ("antialias", true),
            ("alpha", false),
            ("depth", true),
            ("premultipliedAlpha", false),
        ] {
            js_sys::Reflect::set(&opts, &k.into(), &v.into())?;
        }
        js_sys::Reflect::set(&opts, &"powerPreference".into(), &"high-performance".into())?;
        let gl: GL = canvas
            .get_context_with_context_options("webgl2", &opts)?
            .ok_or("WebGL2 is not available in this browser")?
            .dyn_into()?;

        let box_prog = compile(&gl, BOX_VS, BOX_FS)?;
        let glyph_prog = compile(&gl, GLYPH_VS, GLYPH_FS)?;

        let cube_buf = gl.create_buffer().ok_or("no buffer")?;
        gl.bind_buffer(GL::ARRAY_BUFFER, Some(&cube_buf));
        let cube = cube();
        // SAFETY: as in `upload`.
        unsafe {
            gl.buffer_data_with_array_buffer_view(
                GL::ARRAY_BUFFER,
                &js_sys::Float32Array::view(&cube),
                GL::STATIC_DRAW,
            );
        }
        let quad_buf = gl.create_buffer().ok_or("no buffer")?;
        gl.bind_buffer(GL::ARRAY_BUFFER, Some(&quad_buf));
        let quad: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0];
        unsafe {
            gl.buffer_data_with_array_buffer_view(
                GL::ARRAY_BUFFER,
                &js_sys::Float32Array::view(&quad),
                GL::STATIC_DRAW,
            );
        }

        let box_layer = |gl: &GL| -> Result<Layer, JsValue> {
            let vao = gl.create_vertex_array().ok_or("no vao")?;
            gl.bind_vertex_array(Some(&vao));
            gl.bind_buffer(GL::ARRAY_BUFFER, Some(&cube_buf));
            for (loc, off) in [(0u32, 0i32), (1, 12)] {
                gl.enable_vertex_attrib_array(loc);
                gl.vertex_attrib_pointer_with_i32(loc, 3, GL::FLOAT, false, 24, off);
            }
            let buf = gl.create_buffer().ok_or("no buffer")?;
            gl.bind_buffer(GL::ARRAY_BUFFER, Some(&buf));
            let stride = (BOX_FLOATS * 4) as i32;
            for (loc, size, off) in [(2u32, 3, 0), (3, 3, 12), (4, 4, 24), (5, 1, 40), (6, 1, 44)] {
                gl.enable_vertex_attrib_array(loc);
                gl.vertex_attrib_pointer_with_i32(loc, size, GL::FLOAT, false, stride, off);
                gl.vertex_attrib_divisor(loc, 1);
            }
            gl.bind_vertex_array(None);
            Ok(Layer { vao, buf, cap: 0 })
        };
        let glyph_layer = |gl: &GL| -> Result<Layer, JsValue> {
            let vao = gl.create_vertex_array().ok_or("no vao")?;
            gl.bind_vertex_array(Some(&vao));
            gl.bind_buffer(GL::ARRAY_BUFFER, Some(&quad_buf));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_with_i32(0, 2, GL::FLOAT, false, 8, 0);
            let buf = gl.create_buffer().ok_or("no buffer")?;
            gl.bind_buffer(GL::ARRAY_BUFFER, Some(&buf));
            let stride = (GLYPH_FLOATS * 4) as i32;
            for (loc, size, off) in [(1u32, 3, 0), (2, 2, 12), (3, 4, 20), (4, 4, 36), (5, 1, 52)] {
                gl.enable_vertex_attrib_array(loc);
                gl.vertex_attrib_pointer_with_i32(loc, size, GL::FLOAT, false, stride, off);
                gl.vertex_attrib_divisor(loc, 1);
            }
            gl.bind_vertex_array(None);
            Ok(Layer { vao, buf, cap: 0 })
        };

        let (atlas, font) = make_atlas(&gl)?;
        let bg_prog = compile(&gl, BG_VS, BG_FS)?;
        let bg_vao = gl.create_vertex_array().ok_or("no vao")?;
        let u = |p: &WebGlProgram, n: &str| gl.get_uniform_location(p, n);
        Ok(Renderer {
            u_box_vp: u(&box_prog, "u_vp"),
            u_box_fog: u(&box_prog, "u_fog"),
            u_box_fog_color: u(&box_prog, "u_fog_color"),
            u_box_time: u(&box_prog, "u_time"),
            u_gl_vp: u(&glyph_prog, "u_vp"),
            u_gl_screen: u(&glyph_prog, "u_screen"),
            u_gl_fog: u(&glyph_prog, "u_fog"),
            u_gl_tex: u(&glyph_prog, "u_tex"),
            opaque: box_layer(&gl)?,
            clear: box_layer(&gl)?,
            text: glyph_layer(&gl)?,
            glow: glyph_layer(&gl)?,
            hud: glyph_layer(&gl)?,
            box_prog,
            glyph_prog,
            _atlas: atlas,
            u_bg_tex: u(&bg_prog, "u_tex"),
            u_bg_scale: u(&bg_prog, "u_scale"),
            u_bg_base: u(&bg_prog, "u_base"),
            bg_prog,
            bg_vao,
            backdrop: None,
            font,
            gl,
            canvas,
        })
    }

    /// Matches the drawing buffer to the canvas' CSS size times `dpr`.
    /// Returns the drawing-buffer size.
    pub fn resize(&self, css_w: f64, css_h: f64, dpr: f64) -> (u32, u32) {
        let w = (css_w * dpr).round().max(1.0) as u32;
        let h = (css_h * dpr).round().max(1.0) as u32;
        if self.canvas.width() != w || self.canvas.height() != h {
            self.canvas.set_width(w);
            self.canvas.set_height(h);
        }
        (w, h)
    }

    pub fn draw(&mut self, b: &Batches, vp: &Mat4, w: u32, h: u32, fog: [f32; 2], time: f32) {
        let gl = &self.gl;
        gl.viewport(0, 0, w as i32, h as i32);
        gl.clear_color(BG[0], BG[1], BG[2], 1.0);
        gl.clear(GL::COLOR_BUFFER_BIT | GL::DEPTH_BUFFER_BIT);
        if let Some((tex, img_aspect)) = &self.backdrop {
            // Cover-fit: crop the image's long side to the screen's shape.
            let screen = w as f32 / h.max(1) as f32;
            let scale = if screen > *img_aspect {
                [1.0, img_aspect / screen]
            } else {
                [screen / img_aspect, 1.0]
            };
            gl.disable(GL::DEPTH_TEST);
            gl.depth_mask(false);
            gl.use_program(Some(&self.bg_prog));
            gl.active_texture(GL::TEXTURE1);
            gl.bind_texture(GL::TEXTURE_2D, Some(tex));
            gl.uniform1i(self.u_bg_tex.as_ref(), 1);
            gl.uniform2f(self.u_bg_scale.as_ref(), scale[0], scale[1]);
            gl.uniform3f(self.u_bg_base.as_ref(), BG[0], BG[1], BG[2]);
            gl.bind_vertex_array(Some(&self.bg_vao));
            gl.draw_arrays(GL::TRIANGLES, 0, 3);
            gl.active_texture(GL::TEXTURE0);
            gl.depth_mask(true);
        }
        gl.enable(GL::DEPTH_TEST);
        gl.depth_func(GL::LEQUAL);

        upload(gl, &mut self.opaque, &b.opaque);
        upload(gl, &mut self.clear, &b.clear);
        upload(gl, &mut self.text, &b.text);
        upload(gl, &mut self.glow, &b.glow);
        upload(gl, &mut self.hud, &b.hud);

        gl.use_program(Some(&self.box_prog));
        gl.uniform_matrix4fv_with_f32_array(self.u_box_vp.as_ref(), false, vp);
        gl.uniform2f(self.u_box_fog.as_ref(), fog[0], fog[1]);
        gl.uniform3f(self.u_box_fog_color.as_ref(), BG[0], BG[1], BG[2]);
        gl.uniform1f(self.u_box_time.as_ref(), time);
        // Blended too, so the far end of the highway can fade out.
        gl.enable(GL::BLEND);
        gl.blend_func(GL::SRC_ALPHA, GL::ONE_MINUS_SRC_ALPHA);
        gl.depth_mask(true);
        gl.bind_vertex_array(Some(&self.opaque.vao));
        gl.draw_arrays_instanced(GL::TRIANGLES, 0, 36, (b.opaque.len() / BOX_FLOATS) as i32);

        // Notes are blended (they fade in and out) but still write depth, and
        // are emitted far to near, so they sort correctly.
        gl.enable(GL::BLEND);
        gl.blend_func(GL::SRC_ALPHA, GL::ONE_MINUS_SRC_ALPHA);
        gl.bind_vertex_array(Some(&self.clear.vao));
        gl.draw_arrays_instanced(GL::TRIANGLES, 0, 36, (b.clear.len() / BOX_FLOATS) as i32);

        gl.use_program(Some(&self.glyph_prog));
        gl.uniform_matrix4fv_with_f32_array(self.u_gl_vp.as_ref(), false, vp);
        gl.uniform2f(self.u_gl_screen.as_ref(), w as f32, h as f32);
        gl.uniform2f(self.u_gl_fog.as_ref(), fog[0], fog[1]);
        gl.uniform1i(self.u_gl_tex.as_ref(), 0);
        gl.depth_mask(false);
        gl.disable(GL::DEPTH_TEST);
        // Glows add light; the text goes on top of them.
        gl.blend_func(GL::SRC_ALPHA, GL::ONE);
        gl.bind_vertex_array(Some(&self.glow.vao));
        gl.draw_arrays_instanced(GL::TRIANGLES, 0, 6, (b.glow.len() / GLYPH_FLOATS) as i32);
        gl.blend_func(GL::SRC_ALPHA, GL::ONE_MINUS_SRC_ALPHA);
        gl.bind_vertex_array(Some(&self.text.vao));
        gl.draw_arrays_instanced(GL::TRIANGLES, 0, 6, (b.text.len() / GLYPH_FLOATS) as i32);
        gl.disable(GL::DEPTH_TEST);
        gl.bind_vertex_array(Some(&self.hud.vao));
        gl.draw_arrays_instanced(GL::TRIANGLES, 0, 6, (b.hud.len() / GLYPH_FLOATS) as i32);
        gl.bind_vertex_array(None);
        gl.depth_mask(true);
    }
}

impl Renderer {
    /// Sets the backdrop from an image: drawn once, small and blurred, into a
    /// texture, so it costs one texture read per pixel per frame and no more.
    pub fn set_backdrop(&mut self, img: &web_sys::HtmlImageElement) -> Result<(), JsValue> {
        let (iw, ih) = (img.natural_width(), img.natural_height());
        if iw == 0 || ih == 0 {
            return Err("the backdrop image is empty".into());
        }
        let aspect = iw as f32 / ih as f32;
        let (cw, ch) = if aspect >= 1.0 {
            (256, (256.0 / aspect) as u32)
        } else {
            ((256.0 * aspect) as u32, 256)
        };
        let doc = web_sys::window()
            .ok_or("no window")?
            .document()
            .ok_or("no document")?;
        let canvas: HtmlCanvasElement = doc.create_element("canvas")?.dyn_into()?;
        canvas.set_width(cw.max(1));
        canvas.set_height(ch.max(1));
        let ctx: CanvasRenderingContext2d = canvas.get_context("2d")?.ok_or("no 2d")?.dyn_into()?;
        // Browsers without canvas filters still get a soft image from the
        // small size and linear upscaling.
        ctx.set_filter("blur(5px) saturate(1.15)");
        ctx.draw_image_with_html_image_element_and_dw_and_dh(
            img,
            -8.0,
            -8.0,
            cw as f64 + 16.0,
            ch as f64 + 16.0,
        )?;
        let gl = &self.gl;
        let tex = gl.create_texture().ok_or("no texture")?;
        gl.active_texture(GL::TEXTURE1);
        gl.bind_texture(GL::TEXTURE_2D, Some(&tex));
        gl.tex_image_2d_with_u32_and_u32_and_html_canvas_element(
            GL::TEXTURE_2D,
            0,
            GL::RGBA as i32,
            GL::RGBA,
            GL::UNSIGNED_BYTE,
            &canvas,
        )?;
        for (k, v) in [
            (GL::TEXTURE_MIN_FILTER, GL::LINEAR),
            (GL::TEXTURE_MAG_FILTER, GL::LINEAR),
            (GL::TEXTURE_WRAP_S, GL::CLAMP_TO_EDGE),
            (GL::TEXTURE_WRAP_T, GL::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameteri(GL::TEXTURE_2D, k, v as i32);
        }
        gl.active_texture(GL::TEXTURE0);
        self.backdrop = Some((tex, aspect));
        Ok(())
    }
}

/// Draws ASCII 32..128 plus three sprites into a canvas with the system font
/// and uploads it as a mipmapped alpha texture.
fn make_atlas(gl: &GL) -> Result<(WebGlTexture, Font), JsValue> {
    const CELL: u32 = 64;
    let doc = web_sys::window()
        .ok_or("no window")?
        .document()
        .ok_or("no document")?;
    let canvas: HtmlCanvasElement = doc.create_element("canvas")?.dyn_into()?;
    canvas.set_width(CELL * ATLAS_COLS as u32);
    canvas.set_height(CELL * ATLAS_ROWS as u32);
    let ctx: CanvasRenderingContext2d = canvas.get_context("2d")?.ok_or("no 2d")?.dyn_into()?;
    ctx.set_font("700 46px system-ui, -apple-system, \"Segoe UI\", Roboto, sans-serif");
    ctx.set_text_align("center");
    ctx.set_text_baseline("middle");
    ctx.set_fill_style_str("#fff");
    let mut font = Font::default();
    for g in 0..96u32 {
        let ch = char::from_u32(32 + g).unwrap_or(' ').to_string();
        let (cx, cy) = (
            (g % ATLAS_COLS as u32) * CELL,
            (g / ATLAS_COLS as u32) * CELL,
        );
        ctx.fill_text(&ch, (cx + CELL / 2) as f64, (cy + CELL / 2 + 2) as f64)?;
        let w = ctx.measure_text(&ch)?.width();
        font.adv[g as usize] = ((w / CELL as f64) as f32 * 1.02).max(0.2);
    }
    let cell_xy = |c: usize| {
        (
            ((c % ATLAS_COLS) as u32 * CELL) as f64,
            ((c / ATLAS_COLS) as u32 * CELL) as f64,
        )
    };
    let (sx, sy) = cell_xy(CELL_SOLID);
    ctx.fill_rect(sx + 2.0, sy + 2.0, (CELL - 4) as f64, (CELL - 4) as f64);
    let (gx, gy) = cell_xy(CELL_GLOW);
    let r = CELL as f64 / 2.0;
    let grad = ctx.create_radial_gradient(gx + r, gy + r, 0.0, gx + r, gy + r, r - 1.0)?;
    grad.add_color_stop(0.0, "rgba(255,255,255,1)")?;
    grad.add_color_stop(0.35, "rgba(255,255,255,0.55)")?;
    grad.add_color_stop(1.0, "rgba(255,255,255,0)")?;
    ctx.set_fill_style_canvas_gradient(&grad);
    ctx.fill_rect(gx, gy, CELL as f64, CELL as f64);
    let (rx, ry) = cell_xy(CELL_RING);
    ctx.set_stroke_style_str("#fff");
    ctx.set_line_width(4.0);
    ctx.begin_path();
    ctx.arc(rx + r, ry + r, r - 5.0, 0.0, std::f64::consts::TAU)?;
    ctx.stroke();

    let tex = gl.create_texture().ok_or("no texture")?;
    gl.active_texture(GL::TEXTURE0);
    gl.bind_texture(GL::TEXTURE_2D, Some(&tex));
    gl.tex_image_2d_with_u32_and_u32_and_html_canvas_element(
        GL::TEXTURE_2D,
        0,
        GL::RGBA as i32,
        GL::RGBA,
        GL::UNSIGNED_BYTE,
        &canvas,
    )?;
    gl.generate_mipmap(GL::TEXTURE_2D);
    gl.tex_parameteri(
        GL::TEXTURE_2D,
        GL::TEXTURE_MIN_FILTER,
        GL::LINEAR_MIPMAP_LINEAR as i32,
    );
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MAG_FILTER, GL::LINEAR as i32);
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_WRAP_S, GL::CLAMP_TO_EDGE as i32);
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_WRAP_T, GL::CLAMP_TO_EDGE as i32);
    Ok((tex, font))
}
