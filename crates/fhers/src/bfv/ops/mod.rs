//! Operations over ciphertexts

#[cfg(feature = "optimized_ops")]
mod dot_product;

mod mul;

#[cfg(feature = "optimized_ops")]
pub use dot_product::dot_product_scalar;

pub use mul::Multiplicator;

use std::ops::{Add, AddAssign, Mul, MulAssign, Neg, Sub, SubAssign};

use itertools::{izip, Itertools};
use math::rq::{Poly, Representation};

use super::{Ciphertext, Plaintext};

impl Add<&Ciphertext> for &Ciphertext {
	type Output = Ciphertext;

	fn add(self, rhs: &Ciphertext) -> Ciphertext {
		let mut self_clone = self.clone();
		self_clone += rhs;
		self_clone
	}
}

impl AddAssign<&Ciphertext> for Ciphertext {
	fn add_assign(&mut self, rhs: &Ciphertext) {
		assert_eq!(self.par, rhs.par);

		if self.c.is_empty() {
			*self = rhs.clone()
		} else if !rhs.c.is_empty() {
			assert_eq!(self.level, rhs.level);
			assert_eq!(self.c.len(), rhs.c.len());
			izip!(&mut self.c, &rhs.c).for_each(|(c1i, c2i)| *c1i += c2i);
			self.seed = None
		}
	}
}

impl Add<&Plaintext> for &Ciphertext {
	type Output = Ciphertext;

	fn add(self, rhs: &Plaintext) -> Ciphertext {
		let mut self_clone = self.clone();
		self_clone += rhs;
		self_clone
	}
}

impl Add<&Ciphertext> for &Plaintext {
	type Output = Ciphertext;

	fn add(self, rhs: &Ciphertext) -> Ciphertext {
		rhs + self
	}
}

impl AddAssign<&Plaintext> for Ciphertext {
	fn add_assign(&mut self, rhs: &Plaintext) {
		assert_eq!(self.par, rhs.par);
		assert!(!self.c.is_empty());
		assert_eq!(self.level, rhs.level);

		let poly = rhs.to_poly().unwrap();
		self.c[0] += &poly;
		self.seed = None
	}
}

impl Sub<&Ciphertext> for &Ciphertext {
	type Output = Ciphertext;

	fn sub(self, rhs: &Ciphertext) -> Ciphertext {
		let mut self_clone = self.clone();
		self_clone -= rhs;
		self_clone
	}
}

impl SubAssign<&Ciphertext> for Ciphertext {
	fn sub_assign(&mut self, rhs: &Ciphertext) {
		assert_eq!(self.par, rhs.par);

		if self.c.is_empty() {
			*self = -rhs
		} else if !rhs.c.is_empty() {
			assert_eq!(self.level, rhs.level);
			assert_eq!(self.c.len(), rhs.c.len());
			izip!(&mut self.c, &rhs.c).for_each(|(c1i, c2i)| *c1i -= c2i);
			self.seed = None
		}
	}
}

impl Sub<&Plaintext> for &Ciphertext {
	type Output = Ciphertext;

	fn sub(self, rhs: &Plaintext) -> Ciphertext {
		let mut self_clone = self.clone();
		self_clone -= rhs;
		self_clone
	}
}

impl Sub<&Ciphertext> for &Plaintext {
	type Output = Ciphertext;

	fn sub(self, rhs: &Ciphertext) -> Ciphertext {
		-(rhs - self)
	}
}

impl SubAssign<&Plaintext> for Ciphertext {
	fn sub_assign(&mut self, rhs: &Plaintext) {
		assert_eq!(self.par, rhs.par);
		assert!(!self.c.is_empty());
		assert_eq!(self.level, rhs.level);

		let poly = rhs.to_poly().unwrap();
		self.c[0] -= &poly;
		self.seed = None
	}
}

impl Neg for &Ciphertext {
	type Output = Ciphertext;

	fn neg(self) -> Ciphertext {
		let c = self.c.iter().map(|c1i| -c1i).collect_vec();
		Ciphertext {
			par: self.par.clone(),
			seed: None,
			c,
			level: self.level,
		}
	}
}

impl Neg for Ciphertext {
	type Output = Ciphertext;

	fn neg(mut self) -> Ciphertext {
		self.c.iter_mut().for_each(|c1i| *c1i = -&*c1i);
		self.seed = None;
		self
	}
}

impl MulAssign<&Plaintext> for Ciphertext {
	fn mul_assign(&mut self, rhs: &Plaintext) {
		assert_eq!(self.par, rhs.par);
		if !self.c.is_empty() {
			assert_eq!(self.level, rhs.level);
			self.c.iter_mut().for_each(|ci| *ci *= &rhs.poly_ntt);
		}
		self.seed = None
	}
}

impl Mul<&Plaintext> for &Ciphertext {
	type Output = Ciphertext;

	fn mul(self, rhs: &Plaintext) -> Self::Output {
		let mut self_clone = self.clone();
		self_clone *= rhs;
		self_clone
	}
}

impl Mul<&Ciphertext> for &Ciphertext {
	type Output = Ciphertext;

	fn mul(self, rhs: &Ciphertext) -> Self::Output {
		assert_eq!(self.par, rhs.par);

		if self.c.is_empty() {
			return self.clone();
		}
		assert_eq!(self.level, rhs.level);

		let mp = &self.par.mul_params[self.level];

		// Scale all ciphertexts
		// let mut now = std::time::SystemTime::now();
		let self_c = self
			.c
			.iter()
			.map(|ci| ci.scale(&mp.extender_self).unwrap())
			.collect_vec();
		let other_c = rhs
			.c
			.iter()
			.map(|ci| ci.scale(&mp.extender_self).unwrap())
			.collect_vec();
		// println!("Extend: {:?}", now.elapsed().unwrap());

		// Multiply
		// now = std::time::SystemTime::now();
		let mut c = vec![Poly::zero(&mp.to, Representation::Ntt); self_c.len() + other_c.len() - 1];
		for i in 0..self_c.len() {
			for j in 0..other_c.len() {
				c[i + j] += &(&self_c[i] * &other_c[j])
			}
		}
		// println!("Multiply: {:?}", now.elapsed().unwrap());

		// Scale
		// now = std::time::SystemTime::now();
		let c = c
			.iter_mut()
			.map(|ci| {
				ci.change_representation(Representation::PowerBasis);
				let mut ci = ci.scale(&mp.down_scaler).unwrap();
				ci.change_representation(Representation::Ntt);
				ci
			})
			.collect_vec();
		// println!("Scale: {:?}", now.elapsed().unwrap());

		Ciphertext {
			par: self.par.clone(),
			seed: None,
			c,
			level: rhs.level,
		}
	}
}

#[cfg(test)]
mod tests {
	use crate::bfv::{
		encoding::EncodingEnum, BfvParameters, Ciphertext, Encoding, Plaintext, SecretKey,
	};
	use fhers_traits::{FheDecoder, FheDecrypter, FheEncoder, FheEncrypter};
	use std::{error::Error, sync::Arc};

	#[test]
	fn add() -> Result<(), Box<dyn Error>> {
		for params in [
			Arc::new(BfvParameters::default(1, 8)),
			Arc::new(BfvParameters::default(2, 8)),
		] {
			let zero = Ciphertext::zero(&params);
			for _ in 0..50 {
				let a = params.plaintext.random_vec(params.degree());
				let b = params.plaintext.random_vec(params.degree());
				let mut c = a.clone();
				params.plaintext.add_vec(&mut c, &b);

				let sk = SecretKey::random(&params);

				for encoding in [Encoding::poly(), Encoding::simd()] {
					let pt_a = Plaintext::try_encode(&a as &[u64], encoding.clone(), &params)?;
					let pt_b = Plaintext::try_encode(&b as &[u64], encoding.clone(), &params)?;

					let mut ct_a = sk.try_encrypt(&pt_a)?;
					assert_eq!(ct_a, &ct_a + &zero);
					assert_eq!(ct_a, &zero + &ct_a);
					let ct_b = sk.try_encrypt(&pt_b)?;
					let ct_c = &ct_a + &ct_b;
					ct_a += &ct_b;

					let pt_c = sk.try_decrypt(&ct_c)?;
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone())?, c);
					let pt_c = sk.try_decrypt(&ct_a)?;
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone())?, c);
				}
			}
		}

		Ok(())
	}

	#[test]
	fn add_scalar() -> Result<(), Box<dyn Error>> {
		for params in [
			Arc::new(BfvParameters::default(1, 8)),
			Arc::new(BfvParameters::default(2, 8)),
		] {
			for _ in 0..50 {
				let a = params.plaintext.random_vec(params.degree());
				let b = params.plaintext.random_vec(params.degree());
				let mut c = a.clone();
				params.plaintext.add_vec(&mut c, &b);

				let sk = SecretKey::random(&params);

				for encoding in [Encoding::poly(), Encoding::simd()] {
					let zero = Plaintext::zero(encoding.clone(), &params)?;
					let pt_a = Plaintext::try_encode(&a as &[u64], encoding.clone(), &params)?;
					let pt_b = Plaintext::try_encode(&b as &[u64], encoding.clone(), &params)?;

					let mut ct_a = sk.try_encrypt(&pt_a)?;
					assert_eq!(
						Vec::<u64>::try_decode(
							&sk.try_decrypt(&(&ct_a + &zero))?,
							encoding.clone()
						)?,
						a
					);
					assert_eq!(
						Vec::<u64>::try_decode(
							&sk.try_decrypt(&(&zero + &ct_a))?,
							encoding.clone()
						)?,
						a
					);
					let ct_c = &ct_a + &pt_b;
					ct_a += &pt_b;

					let pt_c = sk.try_decrypt(&ct_c)?;
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone())?, c);
					let pt_c = sk.try_decrypt(&ct_a)?;
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone())?, c);
				}
			}
		}

		Ok(())
	}

	#[test]
	fn sub() -> Result<(), Box<dyn Error>> {
		for params in [
			Arc::new(BfvParameters::default(1, 8)),
			Arc::new(BfvParameters::default(2, 8)),
		] {
			let zero = Ciphertext::zero(&params);
			for _ in 0..50 {
				let a = params.plaintext.random_vec(params.degree());
				let mut a_neg = a.clone();
				params.plaintext.neg_vec(&mut a_neg);
				let b = params.plaintext.random_vec(params.degree());
				let mut c = a.clone();
				params.plaintext.sub_vec(&mut c, &b);

				let sk = SecretKey::random(&params);

				for encoding in [Encoding::poly(), Encoding::simd()] {
					let pt_a = Plaintext::try_encode(&a as &[u64], encoding.clone(), &params)?;
					let pt_b = Plaintext::try_encode(&b as &[u64], encoding.clone(), &params)?;

					let mut ct_a = sk.try_encrypt(&pt_a)?;
					assert_eq!(ct_a, &ct_a - &zero);
					assert_eq!(
						Vec::<u64>::try_decode(
							&sk.try_decrypt(&(&zero - &ct_a))?,
							encoding.clone()
						)?,
						a_neg
					);
					let ct_b = sk.try_encrypt(&pt_b)?;
					let ct_c = &ct_a - &ct_b;
					ct_a -= &ct_b;

					let pt_c = sk.try_decrypt(&ct_c)?;
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone())?, c);
					let pt_c = sk.try_decrypt(&ct_a)?;
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone())?, c);
				}
			}
		}

		Ok(())
	}

	#[test]
	fn sub_scalar() -> Result<(), Box<dyn Error>> {
		for params in [
			Arc::new(BfvParameters::default(1, 8)),
			Arc::new(BfvParameters::default(2, 8)),
		] {
			for _ in 0..50 {
				let a = params.plaintext.random_vec(params.degree());
				let mut a_neg = a.clone();
				params.plaintext.neg_vec(&mut a_neg);
				let b = params.plaintext.random_vec(params.degree());
				let mut c = a.clone();
				params.plaintext.sub_vec(&mut c, &b);

				let sk = SecretKey::random(&params);

				for encoding in [Encoding::poly(), Encoding::simd()] {
					let zero = Plaintext::zero(encoding.clone(), &params)?;
					let pt_a = Plaintext::try_encode(&a as &[u64], encoding.clone(), &params)?;
					let pt_b = Plaintext::try_encode(&b as &[u64], encoding.clone(), &params)?;

					let mut ct_a = sk.try_encrypt(&pt_a)?;
					assert_eq!(
						Vec::<u64>::try_decode(
							&sk.try_decrypt(&(&ct_a - &zero))?,
							encoding.clone()
						)?,
						a
					);
					assert_eq!(
						Vec::<u64>::try_decode(
							&sk.try_decrypt(&(&zero - &ct_a))?,
							encoding.clone()
						)?,
						a_neg
					);
					let ct_c = &ct_a - &pt_b;
					ct_a -= &pt_b;

					let pt_c = sk.try_decrypt(&ct_c)?;
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone())?, c);
					let pt_c = sk.try_decrypt(&ct_a)?;
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone())?, c);
				}
			}
		}

		Ok(())
	}

	#[test]
	fn neg() -> Result<(), Box<dyn Error>> {
		for params in [
			Arc::new(BfvParameters::default(1, 8)),
			Arc::new(BfvParameters::default(2, 8)),
		] {
			for _ in 0..50 {
				let a = params.plaintext.random_vec(params.degree());
				let mut c = a.clone();
				params.plaintext.neg_vec(&mut c);

				let sk = SecretKey::random(&params);
				for encoding in [Encoding::poly(), Encoding::simd()] {
					let pt_a = Plaintext::try_encode(&a as &[u64], encoding.clone(), &params)?;

					let ct_a = sk.try_encrypt(&pt_a)?;

					let ct_c = -&ct_a;
					let pt_c = sk.try_decrypt(&ct_c)?;
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone())?, c);

					let ct_c = -ct_a;
					let pt_c = sk.try_decrypt(&ct_c)?;
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone())?, c);
				}
			}
		}

		Ok(())
	}

	#[test]
	fn mul_scalar() -> Result<(), Box<dyn Error>> {
		for params in [
			Arc::new(BfvParameters::default(1, 8)),
			Arc::new(BfvParameters::default(2, 8)),
		] {
			for _ in 0..50 {
				let a = params.plaintext.random_vec(params.degree());
				let b = params.plaintext.random_vec(params.degree());

				let sk = SecretKey::random(&params);
				for encoding in [Encoding::poly(), Encoding::simd()] {
					let mut c = vec![0u64; params.degree()];
					match encoding.encoding {
						EncodingEnum::Poly => {
							for i in 0..params.degree() {
								for j in 0..params.degree() {
									if i + j >= params.degree() {
										c[(i + j) % params.degree()] = params.plaintext.sub(
											c[(i + j) % params.degree()],
											params.plaintext.mul(a[i], b[j]),
										);
									} else {
										c[i + j] = params
											.plaintext
											.add(c[i + j], params.plaintext.mul(a[i], b[j]));
									}
								}
							}
						}
						EncodingEnum::Simd => {
							c = a.clone();
							params.plaintext.mul_vec(&mut c, &b);
						}
					}

					let pt_a = Plaintext::try_encode(&a as &[u64], encoding.clone(), &params)?;
					let pt_b = Plaintext::try_encode(&b as &[u64], encoding.clone(), &params)?;

					let mut ct_a = sk.try_encrypt(&pt_a)?;
					let ct_c = &ct_a * &pt_b;
					ct_a *= &pt_b;

					let pt_c = sk.try_decrypt(&ct_c)?;
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone())?, c);
					let pt_c = sk.try_decrypt(&ct_a)?;
					assert_eq!(Vec::<u64>::try_decode(&pt_c, encoding.clone())?, c);
				}
			}
		}

		Ok(())
	}

	#[test]
	fn mul() -> Result<(), Box<dyn Error>> {
		let par = Arc::new(BfvParameters::default(2, 8));
		for _ in 0..50 {
			// We will encode `values` in an Simd format, and check that the product is
			// computed correctly.
			let values = par.plaintext.random_vec(par.degree());
			let mut expected = values.clone();
			par.plaintext.mul_vec(&mut expected, &values);

			let sk = SecretKey::random(&par);
			let pt = Plaintext::try_encode(&values as &[u64], Encoding::simd(), &par)?;

			let ct1 = sk.try_encrypt(&pt)?;
			let ct2 = sk.try_encrypt(&pt)?;
			let ct3 = &ct1 * &ct2;
			let ct4 = &ct3 * &ct3;

			println!("Noise: {}", unsafe { sk.measure_noise(&ct3)? });
			let pt = sk.try_decrypt(&ct3)?;
			assert_eq!(Vec::<u64>::try_decode(&pt, Encoding::simd())?, expected);

			let e = expected.clone();
			par.plaintext.mul_vec(&mut expected, &e);
			println!("Noise: {}", unsafe { sk.measure_noise(&ct4)? });
			let pt = sk.try_decrypt(&ct4)?;
			assert_eq!(Vec::<u64>::try_decode(&pt, Encoding::simd())?, expected);
		}
		Ok(())
	}
}
