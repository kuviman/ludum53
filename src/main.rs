use geng::prelude::*;

mod camera;
mod draw3d;

use camera::*;
use draw3d::Draw3d;

#[derive(Deserialize)]
pub struct Config {
    pub sky_color: Rgba<f32>,
    pub gravity: f32,
    pub throw_speed: f32,
    pub throw_angle: f32,
    pub item_scale: f32,
    pub item_hold_scale: f32,
    pub hand_radius: f32,
    pub item_max_w: f32,
    pub throw_target_height: f32,
    pub ui_fov: f32,
    pub fov: f32,
    pub earth_radius: f32,
    pub ride_speed: f32,
    pub camera_height: f32,
    pub camera_rot: f32,
    pub road_width: f32,
    pub mailbox_size: f32,
    pub distance_between_mailboxes: f32,
}

#[derive(geng::asset::Load)]
pub struct Shaders {
    pub sprite: ugli::Program,
    pub mesh3d: ugli::Program,
}

#[derive(Deref, DerefMut)]
struct Texture(#[deref] ugli::Texture);

impl std::borrow::Borrow<ugli::Texture> for &Texture {
    fn borrow(&self) -> &ugli::Texture {
        &self.0
    }
}

impl geng::asset::Load for Texture {
    fn load(manager: &geng::Manager, path: &std::path::Path) -> geng::asset::Future<Self> {
        let texture = manager.load(path);
        async move {
            let mut texture: ugli::Texture = texture.await?;
            texture.set_filter(ugli::Filter::Nearest);
            Ok(Self(texture))
        }
        .boxed_local()
    }

    const DEFAULT_EXT: Option<&'static str> = ugli::Texture::DEFAULT_EXT;
}

#[derive(geng::asset::Load)]
pub struct Assets {
    shaders: Shaders,
    envelope: Rc<Texture>,
    bag: Texture,
    hand: Texture,
    holding_hand: Texture,
    mailbox: Texture,
    #[load(postprocess = "make_repeated")]
    road: Texture,
}

fn make_repeated(texture: &mut Texture) {
    texture.set_wrap_mode(ugli::WrapMode::Repeat);
}

struct Item {
    texture: Rc<Texture>,
    pos: vec2<f32>,
    vel: vec2<f32>,
    rot: f32,
    w: f32,
    half_size: vec2<f32>,
}

impl Item {
    pub fn new(texture: &Rc<Texture>, scale: f32) -> Self {
        Self {
            texture: texture.clone(),
            pos: vec2::ZERO,
            vel: vec2::ZERO,
            rot: thread_rng().gen_range(0.0..2.0 * f32::PI),
            w: 0.0,
            half_size: vec2(texture.size().map(|x| x as f32).aspect(), 1.0) * scale,
        }
    }
}

struct Mailbox {
    pub x: f32,
    pub latitude: f32,
}

struct Game {
    framebuffer_size: vec2<f32>,
    geng: Geng,
    assets: Rc<Assets>,
    config: Rc<Config>,
    camera: Camera,
    items: Vec<Item>,
    bag_position: Aabb2<f32>,
    holding: Option<Item>,
    mailboxes: Vec<Mailbox>,
    draw3d: Draw3d,
    my_latitude: f32,
    road_mesh: ugli::VertexBuffer<draw3d::Vertex>,
}

impl Game {
    pub fn new(geng: &Geng, assets: &Rc<Assets>, config: &Rc<Config>) -> Self {
        let camera = Camera::new(
            config.fov.to_radians(),
            config.ui_fov,
            config.camera_rot.to_radians(),
            config.earth_radius + config.camera_height,
        );
        Self {
            framebuffer_size: vec2::splat(1.0),
            geng: geng.clone(),
            assets: assets.clone(),
            config: config.clone(),
            bag_position: Aabb2::point(vec2(0.0, -camera.fov() / 2.0 + 1.0)).extend_uniform(1.0),
            camera,
            items: vec![],
            holding: None,
            mailboxes: vec![],
            draw3d: Draw3d::new(geng, assets),
            my_latitude: 0.0,
            road_mesh: ugli::VertexBuffer::new_static(geng.ugli(), {
                const N: usize = 100;
                (0..=N)
                    .flat_map(|i| {
                        let yz = vec2(config.earth_radius, 0.0)
                            .rotate(2.0 * f32::PI * i as f32 / N as f32);
                        let uv_y =
                            (2.0 * f32::PI * config.earth_radius).ceil() * i as f32 / N as f32;
                        [-1, 1].map(|x| draw3d::Vertex {
                            a_pos: vec3(x as f32 * config.road_width, yz.x, yz.y),
                            a_uv: vec2(x as f32 * 0.5 + 0.5, uv_y),
                        })
                    })
                    .collect()
            }),
        }
    }
}

impl geng::State for Game {
    fn handle_event(&mut self, event: geng::Event) {
        match event {
            geng::Event::MouseDown {
                position,
                button: geng::MouseButton::Left,
            } => {
                let pos = self
                    .camera
                    .as_2d()
                    .screen_to_world(self.framebuffer_size, position.map(|x| x as f32));
                if let Some(index) = self.items.iter().rposition(|item| {
                    Aabb2::ZERO.extend_uniform(1.0).contains(
                        (Quad::unit()
                            .scale(item.half_size.map(|x| x + self.config.hand_radius))
                            .rotate(item.rot)
                            .translate(item.pos)
                            .transform
                            .inverse()
                            * pos.extend(1.0))
                        .into_2d(),
                    )
                }) {
                    self.holding = Some(self.items.remove(index));
                } else if self
                    .bag_position
                    .extend_uniform(self.config.hand_radius)
                    .contains(pos)
                {
                    self.holding = Some(Item::new(&self.assets.envelope, self.config.item_scale));
                }
            }
            geng::Event::MouseUp {
                position,
                button: geng::MouseButton::Left,
            } => {
                let pos = self
                    .camera
                    .as_2d()
                    .screen_to_world(self.framebuffer_size, position.map(|x| x as f32));
                if let Some(mut item) = self.holding.take() {
                    item.pos = pos;
                    item.vel = (vec2(0.0, self.config.throw_target_height) - item.pos)
                        .normalize_or_zero()
                        .rotate(thread_rng().gen_range(
                            -self.config.throw_angle.to_radians()
                                ..self.config.throw_angle.to_radians(),
                        ))
                        * self.config.throw_speed;
                    item.w = thread_rng().gen_range(-1.0..1.0) * self.config.item_max_w;
                    self.items.push(item);
                }
            }
            _ => {}
        }
    }
    fn update(&mut self, delta_time: f64) {
        let delta_time = delta_time as f32;

        for item in &mut self.items {
            item.vel.y -= self.config.gravity * delta_time;
            item.pos += item.vel * delta_time;
            item.rot += item.w * delta_time;
        }

        self.my_latitude += self.config.ride_speed * delta_time;

        self.mailboxes
            .retain(|mailbox| mailbox.latitude > self.my_latitude - f32::PI);
        while self.mailboxes.last().map_or(true, |mailbox| {
            mailbox.latitude < self.my_latitude + f32::PI
        }) {
            let last_latitude = self
                .mailboxes
                .last()
                .map_or(self.my_latitude, |mailbox| mailbox.latitude);
            for x in [-1, 1] {
                self.mailboxes.push(Mailbox {
                    x: x as f32 * (self.config.road_width + self.config.mailbox_size / 2.0),
                    latitude: last_latitude + self.config.distance_between_mailboxes.to_radians(),
                });
            }
        }
    }
    fn draw(&mut self, framebuffer: &mut ugli::Framebuffer) {
        self.framebuffer_size = framebuffer.size().map(|x| x as f32);
        self.camera.latitude = self.my_latitude;
        ugli::clear(framebuffer, Some(self.config.sky_color), Some(1.0), None);
        self.draw3d.draw(
            framebuffer,
            &self.camera,
            &self.road_mesh,
            ugli::DrawMode::TriangleStrip,
            &self.assets.road,
        );

        let mouse_pos = self.camera.as_2d().screen_to_world(
            self.framebuffer_size,
            self.geng.window().cursor_position().map(|x| x as f32),
        );

        for mailbox in &self.mailboxes {
            let circle_pos = vec2(self.config.earth_radius, 0.0).rotate(mailbox.latitude);
            self.draw3d.draw_sprite(
                framebuffer,
                &self.camera,
                &self.assets.mailbox,
                vec3(mailbox.x, circle_pos.x, -circle_pos.y),
                vec2::splat(self.config.mailbox_size),
            );
        }

        self.geng.draw2d().draw2d(
            framebuffer,
            self.camera.as_2d(),
            &draw2d::TexturedQuad::new(self.bag_position, &self.assets.bag),
        );
        if let Some(item) = &self.holding {
            self.geng.draw2d().draw2d(
                framebuffer,
                self.camera.as_2d(),
                &draw2d::TexturedQuad::unit(&*item.texture)
                    .scale(item.half_size * self.config.item_hold_scale)
                    .rotate(item.rot)
                    .translate(mouse_pos),
            );
        }
        for item in &self.items {
            self.geng.draw2d().draw2d(
                framebuffer,
                self.camera.as_2d(),
                &draw2d::TexturedQuad::unit(&*item.texture)
                    .scale(item.half_size)
                    .rotate(item.rot)
                    .translate(item.pos),
            );
        }

        self.geng.draw2d().draw2d(
            framebuffer,
            self.camera.as_2d(),
            &draw2d::TexturedQuad::unit(if self.holding.is_some() {
                &self.assets.holding_hand
            } else {
                &self.assets.hand
            })
            .scale_uniform(self.config.hand_radius)
            .translate(mouse_pos),
        );
    }
}

fn main() {
    let geng = Geng::new("Ludum53");
    geng.clone().run_loading(async move {
        let assets: Rc<Assets> = geng
            .asset_manager()
            .load(run_dir().join("assets"))
            .await
            .unwrap();
        let config: Config = file::load_detect(run_dir().join("assets").join("config.toml"))
            .await
            .unwrap();
        let config = Rc::new(config);
        Game::new(&geng, &assets, &config)
    })
}
