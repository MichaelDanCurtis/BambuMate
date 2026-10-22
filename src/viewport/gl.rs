//! WebGL2 renderer for the model viewport: dots or wireframe, one colour,
//! drawn only when asked. Interaction and pins live in `mod.rs`.

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    HtmlCanvasElement, WebGl2RenderingContext as Gl, WebGlBuffer, WebGlProgram, WebGlShader,
    WebGlUniformLocation, WebGlVertexArrayObject,
};

const VERTEX: &str = r#"#version 300 es
in vec3 a_pos;
uniform mat4 u_vp;
uniform float u_size;
out float v_depth;
void main() {
  vec4 p = u_vp * vec4(a_pos, 1.0);
  gl_Position = p;
  gl_PointSize = u_size;
  v_depth = clamp(p.z / p.w * 0.5 + 0.5, 0.0, 1.0);
}
"#;

const FRAGMENT: &str = r#"#version 300 es
precision mediump float;
in float v_depth;
uniform vec3 u_color;
uniform float u_fade;
out vec4 o;
void main() {
  // Nearer dots are brighter; u_fade is 0 for lines (no fade).
  float a = mix(1.0, 0.35, v_depth * u_fade);
  o = vec4(u_color, a);
}
"#;

/// `a_pos` is bound to this attribute slot before linking.
const POS: u32 = 0;
/// Dot diameter in CSS pixels.
const DOT_SIZE: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Dots,
    Wireframe,
}

impl Mode {
    /// `data-mode` on the viewport.
    pub fn key(self) -> &'static str {
        match self {
            Self::Dots => "dots",
            Self::Wireframe => "wireframe",
        }
    }
}

/// Linear RGB in 0..=1, from the theme tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colors {
    pub model: [f32; 3],
    pub background: [f32; 3],
}

/// `#rrggbb` or `#rgb` (surrounding space allowed) to unit RGB.
pub fn parse_hex_rgb(s: &str) -> Option<[f32; 3]> {
    let hex = s.trim().strip_prefix('#')?;
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize, len: usize| u8::from_str_radix(&hex[i * len..(i + 1) * len], 16).ok();
    let rgb = match hex.len() {
        6 => [channel(0, 2)?, channel(1, 2)?, channel(2, 2)?],
        // #abc is #aabbcc.
        3 => [
            channel(0, 1)? * 17,
            channel(1, 1)? * 17,
            channel(2, 1)? * 17,
        ],
        _ => return None,
    };
    Some(rgb.map(|c| f32::from(c) / 255.0))
}

/// The dark theme's value, used if a token cannot be read.
fn fallback_rgb(var: &str) -> [f32; 3] {
    match var {
        "--nd-black" => [0.0, 0.0, 0.0],
        // --nd-text-primary, #e8e8e8.
        _ => [232.0 / 255.0; 3],
    }
}

/// A `--nd-*` hex token from `<html>`'s computed style, read now so a theme
/// switch shows on the next draw.
pub fn css_rgb(var: &str) -> [f32; 3] {
    web_sys::window()
        .and_then(|w| {
            let root = w.document()?.document_element()?;
            w.get_computed_style(&root).ok().flatten()
        })
        .and_then(|style| style.get_property_value(var).ok())
        .and_then(|v| parse_hex_rgb(&v))
        .unwrap_or_else(|| fallback_rgb(var))
}

/// One vertex buffer of xyz positions and the VAO that reads it.
struct Mesh {
    vao: WebGlVertexArrayObject,
    buffer: WebGlBuffer,
    count: i32,
}

impl Mesh {
    fn new(gl: &Gl) -> Option<Mesh> {
        let vao = gl.create_vertex_array()?;
        let buffer = gl.create_buffer()?;
        gl.bind_vertex_array(Some(&vao));
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&buffer));
        gl.enable_vertex_attrib_array(POS);
        gl.vertex_attrib_pointer_with_i32(POS, 3, Gl::FLOAT, false, 0, 0);
        gl.bind_vertex_array(None);
        Some(Mesh {
            vao,
            buffer,
            count: 0,
        })
    }

    fn upload(&mut self, gl: &Gl, xyz: &[f32]) {
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&self.buffer));
        // Copies into a JS array, which GL copies again; no unsafe view.
        let data = js_sys::Float32Array::from(xyz);
        gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &data, Gl::STATIC_DRAW);
        self.count = i32::try_from(xyz.len() / 3).unwrap_or(i32::MAX);
    }

    fn draw(&self, gl: &Gl, primitive: u32) {
        if self.count == 0 {
            return;
        }
        gl.bind_vertex_array(Some(&self.vao));
        gl.draw_arrays(primitive, 0, self.count);
        gl.bind_vertex_array(None);
    }

    fn delete(&self, gl: &Gl) {
        gl.delete_vertex_array(Some(&self.vao));
        gl.delete_buffer(Some(&self.buffer));
    }
}

/// Why there is no renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlProblem {
    /// The browser gave no WebGL2 context.
    NoWebGl,
    /// There is a context, but the program or buffers could not be built.
    SetupFailed,
}

pub struct Renderer {
    canvas: HtmlCanvasElement,
    gl: Gl,
    program: WebGlProgram,
    u_vp: Option<WebGlUniformLocation>,
    u_size: Option<WebGlUniformLocation>,
    u_color: Option<WebGlUniformLocation>,
    u_fade: Option<WebGlUniformLocation>,
    dots: Mesh,
    edges: Mesh,
}

impl Renderer {
    /// `NoWebGl` when the canvas cannot give a WebGL2 context; `SetupFailed`
    /// when it can but the shaders or buffers do not build (the reason goes to
    /// the console).
    pub fn new(canvas: &HtmlCanvasElement) -> Result<Renderer, GlProblem> {
        let gl = context(canvas).ok_or(GlProblem::NoWebGl)?;
        let program = match link(&gl) {
            Ok(p) => p,
            Err(e) => {
                web_sys::console::warn_1(&format!("viewport: {e}").into());
                return Err(GlProblem::SetupFailed);
            }
        };
        let dots = Mesh::new(&gl).ok_or(GlProblem::SetupFailed)?;
        let edges = Mesh::new(&gl).ok_or(GlProblem::SetupFailed)?;
        // Dots blend into each other, so no depth test.
        gl.disable(Gl::DEPTH_TEST);
        gl.enable(Gl::BLEND);
        gl.blend_func(Gl::SRC_ALPHA, Gl::ONE_MINUS_SRC_ALPHA);
        Ok(Renderer {
            canvas: canvas.clone(),
            u_vp: gl.get_uniform_location(&program, "u_vp"),
            u_size: gl.get_uniform_location(&program, "u_size"),
            u_color: gl.get_uniform_location(&program, "u_color"),
            u_fade: gl.get_uniform_location(&program, "u_fade"),
            gl,
            program,
            dots,
            edges,
        })
    }

    /// The canvas this renderer draws to.
    pub fn canvas(&self) -> &HtmlCanvasElement {
        &self.canvas
    }

    /// Replace the geometry: `dots` are xyz triples, `edges` are pairs of them.
    pub fn set_geometry(&mut self, dots: &[f32], edges: &[f32]) {
        self.dots.upload(&self.gl, dots);
        self.edges.upload(&self.gl, edges);
    }

    /// Size the backing store to the canvas's CSS size times `dpr`, clear to
    /// the background and draw the model once.
    pub fn draw(&self, view_proj: &[f32; 16], mode: Mode, colors: Colors, dpr: f32) {
        let gl = &self.gl;
        let scale = |css: i32| ((css.max(0) as f32) * dpr).round().max(1.0) as u32;
        let (w, h) = (
            scale(self.canvas.client_width()),
            scale(self.canvas.client_height()),
        );
        // Setting the size, even to the same value, reallocates the buffer.
        if self.canvas.width() != w {
            self.canvas.set_width(w);
        }
        if self.canvas.height() != h {
            self.canvas.set_height(h);
        }
        gl.viewport(0, 0, w as i32, h as i32);

        let [r, g, b] = colors.background;
        gl.clear_color(r, g, b, 1.0);
        gl.clear(Gl::COLOR_BUFFER_BIT);

        gl.use_program(Some(&self.program));
        gl.uniform_matrix4fv_with_f32_array(self.u_vp.as_ref(), false, view_proj);
        let [r, g, b] = colors.model;
        gl.uniform3f(self.u_color.as_ref(), r, g, b);
        match mode {
            Mode::Dots => {
                gl.uniform1f(self.u_size.as_ref(), DOT_SIZE * dpr);
                gl.uniform1f(self.u_fade.as_ref(), 1.0);
                self.dots.draw(gl, Gl::POINTS);
            }
            Mode::Wireframe => {
                gl.uniform1f(self.u_size.as_ref(), 1.0);
                gl.uniform1f(self.u_fade.as_ref(), 0.0);
                self.edges.draw(gl, Gl::LINES);
            }
        }
    }
}

impl Drop for Renderer {
    /// Free the GL objects and give the context back now rather than at
    /// garbage collection, so reloading models never hits the browser's
    /// live-context limit.
    fn drop(&mut self) {
        self.dots.delete(&self.gl);
        self.edges.delete(&self.gl);
        self.gl.delete_program(Some(&self.program));
        if let Ok(Some(ext)) = self.gl.get_extension("WEBGL_lose_context") {
            if let Ok(lose) = js_sys::Reflect::get(&ext, &"loseContext".into()) {
                if let Some(f) = lose.dyn_ref::<js_sys::Function>() {
                    let _ = f.call0(&ext);
                }
            }
        }
    }
}

/// An opaque, antialiased WebGL2 context with no depth buffer.
fn context(canvas: &HtmlCanvasElement) -> Option<Gl> {
    let opts = js_sys::Object::new();
    for (key, value) in [("alpha", false), ("antialias", true), ("depth", false)] {
        let _ = js_sys::Reflect::set(&opts, &key.into(), &JsValue::from_bool(value));
    }
    canvas
        .get_context_with_context_options("webgl2", &opts)
        .ok()
        .flatten()?
        .dyn_into::<Gl>()
        .ok()
}

fn compile(gl: &Gl, kind: u32, src: &str) -> Result<WebGlShader, String> {
    let shader = gl.create_shader(kind).ok_or("could not create a shader")?;
    gl.shader_source(&shader, src);
    gl.compile_shader(&shader);
    if gl
        .get_shader_parameter(&shader, Gl::COMPILE_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        Ok(shader)
    } else {
        let log = gl.get_shader_info_log(&shader).unwrap_or_default();
        gl.delete_shader(Some(&shader));
        Err(format!("shader did not compile: {log}"))
    }
}

fn link(gl: &Gl) -> Result<WebGlProgram, String> {
    let vs = compile(gl, Gl::VERTEX_SHADER, VERTEX)?;
    let fs = match compile(gl, Gl::FRAGMENT_SHADER, FRAGMENT) {
        Ok(fs) => fs,
        Err(e) => {
            gl.delete_shader(Some(&vs));
            return Err(e);
        }
    };
    let program = gl.create_program().ok_or("could not create a program")?;
    gl.attach_shader(&program, &vs);
    gl.attach_shader(&program, &fs);
    gl.bind_attrib_location(&program, POS, "a_pos");
    gl.link_program(&program);
    // The program keeps what it needs once linked.
    gl.delete_shader(Some(&vs));
    gl.delete_shader(Some(&fs));
    if gl
        .get_program_parameter(&program, Gl::LINK_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        Ok(program)
    } else {
        let log = gl.get_program_info_log(&program).unwrap_or_default();
        gl.delete_program(Some(&program));
        Err(format!("program did not link: {log}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_tokens_parse_to_unit_rgb() {
        assert_eq!(parse_hex_rgb("#ffffff"), Some([1.0, 1.0, 1.0]));
        assert_eq!(parse_hex_rgb(" #000000 "), Some([0.0, 0.0, 0.0]));
        assert_eq!(parse_hex_rgb("rgb(1,2,3)"), None);
    }

    #[test]
    fn hex_parsing_handles_case_short_form_and_junk() {
        let [r, g, b] = parse_hex_rgb("#E8e8E8").unwrap();
        assert!((r - 232.0 / 255.0).abs() < 1e-6 && r == g && g == b);
        assert_eq!(parse_hex_rgb("#fff"), Some([1.0, 1.0, 1.0]));
        for bad in ["", "#", "ffffff", "#fffff", "#gggggg", "#ffffff00", "#ffé"] {
            assert_eq!(parse_hex_rgb(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn modes_have_distinct_keys() {
        assert_eq!(
            [Mode::Dots, Mode::Wireframe].map(Mode::key),
            ["dots", "wireframe"]
        );
    }

    #[test]
    fn unreadable_tokens_fall_back_to_the_dark_theme() {
        assert_eq!(fallback_rgb("--nd-black"), [0.0; 3]);
        assert_eq!(
            fallback_rgb("--nd-text-primary"),
            parse_hex_rgb("#e8e8e8").unwrap()
        );
    }
}
